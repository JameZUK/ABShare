# ABShare — Specification

_Draft · 2026-10-17_

ABShare is a small, self-hosted server for sharing an audiobook library with a few invited people over the internet. You listen in an installable web app (PWA) on a phone, tablet or computer. Your place in every book follows you between devices. Books can be downloaded for offline listening in the app or saved as files. It is one Rust binary with an SQLite database. It sits behind an ordinary reverse proxy and treats the library as read-only.

**Security is the first requirement.** ABShare will face the internet, so every design choice in this document is weighed against attack surface first and convenience second. Where they conflict, security wins, and the trade-off is recorded in §1.

**v1 includes:**
- the server: a single Rust binary that scans the library, serves audio and the app, and owns accounts;
- a web app (an installable PWA) with a lock-screen-friendly player, chapters, bookmarks, a sleep timer, speed control and offline downloads;
- admin tools in the web app plus a CLI for bootstrap and recovery;
- a Docker image and a hardened systemd unit.

Native apps are out of scope for v1. The API is documented and versioned so that they can come later (§10).

---

## 1. Decisions

| # | Decision | Choice |
|---|---|---|
| D1 | Who uses it | **Invite-only.** An admin sends a single-use invite link. There is no self sign-up. Each person has their own account, progress and devices. It is sized for about 2–50 users. Decided 2026-10-17. |
| D2 | Sign-in | **Passkeys (WebAuthn), or password + TOTP.** A passkey with user verification is enough on its own. A password always needs a second factor (TOTP, or a passkey as the second step). Recovery codes are issued at enrolment. **A second factor is required for everyone.** An admin setting can relax this for listeners only, and the setting is audited and shown as a warning on the admin home page. Decided 2026-10-17. |
| D3 | The library | **Read-only.** The server never writes, renames or deletes anything under a library root. It opens files `O_RDONLY`, and deployment mounts the library read-only (`:ro`, `ReadOnlyPaths=`). Edited metadata, covers, playable copies and caches live in the server's own data directory. Decided 2026-10-17. |
| D4 | Repository | **Public, GPL-2.0-or-later, with lyrion-anywhere's standing rule:** no personal or homelab information in the repo (§14). Decided 2026-10-17. |
| D5 | Server stack | **Rust (edition 2024), axum 0.8 on tokio/hyper 1, SQLite via rusqlite (bundled).** This is the stack proven in lyrion-anywhere: memory-safe, no GC, and about 10 MB RSS at idle. `#![forbid(unsafe_code)]` in every crate of ours. Decided 2026-10-17. |
| D6 | Core / host split | **`core` does no I/O.** Metadata normalisation, book assembly (which files make a book), chapter and position maths, the progress-merge rules, input validation and access policy are pure functions with exhaustive tests and fuzz targets. The `server` crate does all I/O. Decided 2026-10-17. |
| D7 | Unit of playback | **Every book plays as one file on one timeline.** A book that is a single browser-safe file is served as it is (direct play). A multi-file book, or a codec browsers can't play, gets a *playable copy*: one AAC-LC `.m4b` with chapters, built in the background and cached in the data dir. Positions, chapters, downloads and the Media Session all work on one timeline in milliseconds. The only on-demand transcoding is data-saver segments (D23), which are small, cached, deduplicated and capped, so a listener can never make the server do unbounded ffmpeg work. Decided 2026-10-17; data-saver exception added 2026-10-17. |
| D8 | Streaming transport | **A plain `<audio>` element with HTTP Range (206) requests.** It does not use MSE/HLS. AAC-LC/MP3 in a fast-start container plays natively everywhere, keeps playing with the screen locked on iOS and Android, and avoids MSE codec quirks. Decided 2026-10-17. |
| D9 | Media authentication | **A media cookie bound to the device session.** `<audio src>` can't send a bearer header. Media requests carry `abshare_media` (`HttpOnly; Secure; SameSite=Strict; Path=/v1/media`): an opaque, hashed-at-rest, rolling 12 h token that is revoked with the device. **No tokens in URLs**, so none leak into logs, history or Referer. Decided 2026-10-17. |
| D10 | ffmpeg | **ffmpeg/ffprobe 7.1 or later, a minimal build, required, invoked only through one hardened wrapper and sandbox (D19).** It is bundled in the Docker image, built with only the demuxers, decoders and encoders ABShare needs and no network protocols. For systemd installs the path is configured, and the version and the absence of network protocols are checked at start-up. 7.1 is the first release that decodes xHE-AAC (USAC), which some store-bought audiobooks use. Decided 2026-10-17. |
| D11 | Metadata source | **Embedded tags first, then sidecars, then folder and file names.** The server makes **no outbound network connections at all** in v1: no metadata services, no update checks, no telemetry. Admins can override fields in the data dir. Decided 2026-10-17. |
| D12 | Progress sync | **The server holds each user's position per book. The newest listening event wins, with bounds.** Clients send progress while playing and queue it while offline. The server refuses events from the future, very old events and duplicates (§5.6). Decided 2026-10-17. |
| D13 | Offline in the app | **IndexedDB, as chunked Blobs partitioned per user.** This is lyrion-anywhere's proven approach; OPFS writes aren't available from the page in Safari 17. How playback reads them (one composed Blob URL, or a service-worker Range responder) is a Phase 0 spike (§11). Decided 2026-10-17. |
| D14 | Web UI | **SvelteKit 2 + Svelte 5, static build, hash router, no third-party runtime code.** A strict CSP with no inline scripts and no `unsafe-eval`, enforced at build time. This keeps parity with lyrion-anywhere, so its CSP externaliser, service-worker policy and HTML-injection ban tests can be reused. Decided 2026-10-17. |
| D15 | Tokens | **Opaque random tokens; no JWTs.** The server is the only verifier, so signed tokens would add a format to get wrong and gain nothing. A short-lived access token is held in memory only. The refresh token is rotated on every use, with reuse detection, in an `HttpOnly` cookie. This is lyrion-anywhere's model unchanged, including its 60 s reuse grace. Decided 2026-10-17. |
| D16 | Configuration | **A TOML config file for deployment settings (`deny_unknown_fields`, validated at start-up, refused if world-writable) and the database for runtime settings that admins change in the UI.** CLI flags override the file. Decided 2026-10-17. |
| D17 | Stable IDs | **A book's ID is HMAC-SHA256(server secret, key), truncated to 128 bits and base32-encoded.** The key is `asin:<ASIN>` when one is known, otherwise `path:<library id>/<relative folder>`. IDs survive rescans and (with an ASIN) folder renames, are unguessable, and never reveal paths. Progress, bookmarks and downloads all hang off this ID. Decided 2026-10-17. |
| D18 | Library change detection | **Polling, not inotify.** Libraries are often NFS mounts, where inotify sees nothing. The server does a cheap metadata walk (size + mtime) on a schedule (default every 6 h), at start-up and when an admin asks. Only new or changed files are probed. Decided 2026-10-17. |
| D19 | ffmpeg sandbox | **Every ffmpeg/ffprobe run goes through `abshare-jail`**, a tiny helper binary of ours. Before it replaces itself with ffmpeg (`execve`), it applies Linux **Landlock** at ABI ≥ 6 as a hard requirement:
- read access to the named inputs only;
- write access to the job directory only;
- **execute rights on the named program alone**, so ffmpeg can't start anything else;
- no TCP bind or connect;
- no signals or abstract Unix sockets outside the sandbox.

It also applies **seccomp**, `PR_SET_NO_NEW_PRIVS` and rlimits (address space, CPU seconds, file size, open files, no core dumps), and **refuses to run if it inherits any file descriptor above 2**, because an inherited descriptor would bypass Landlock. The seccomp filters are two stacked layers. First, `clone3` returns ENOSYS so the C library falls back to `clone`, whose namespace flags a filter can see. Second, a deny-list covers non-`AF_UNIX` sockets, namespaces, `ptrace`, mounts, io_uring, BPF, `memfd_create`/`execveat`, module and kexec loading, and clock and quota changes. Phase 1 replaces the deny-list with an allow-list tuned to the bundled minimal ffmpeg. ffmpeg 7's threaded pipeline needs an address-space limit of about 2 GiB, which is virtual reservation, not memory use. **Built and tested 2026-10-17** (`jail/`): 14 escape tests cover reads outside the inputs, writes outside the job directory, sockets, starting other programs, signals, the file-size limit, inherited descriptors and bad arguments. Removing each protection in turn makes at least one test fail. Real ffmpeg and ffprobe run correctly inside it, while a jailed ffmpeg asked to fetch a URL or read `/etc/passwd` is refused. Media parsers are the largest attack surface in any media server, so a crafted file that compromises ffmpeg must find itself unable to read anything else, write anywhere useful, or reach the network. If the kernel lacks Landlock, the server **refuses to start**, unless `allow_unsandboxed_ffmpeg = true` is set, which logs a warning on every start and shows it on the admin home page. Decided 2026-10-17. |
| D20 | Security target | **OWASP ASVS 5.0 Level 2 throughout, with Level 3 controls for authentication, session management, file handling and configuration.** Every ASVS requirement in scope is mapped to code and tests in `docs/security/asvs.md`. Decided 2026-10-17. |
| D21 | Fuzzing | **Real coverage-guided fuzzing with `cargo-fuzz`/libFuzzer**, as well as seeded robustness loops in ordinary tests. Every parser of untrusted bytes has a target. CI runs each target for 60 s on every push and for 30 min nightly. Crashes become regression tests. This answers lyrion-anywhere's review finding M18, which found its docs over-claimed fuzzing. Decided 2026-10-17. |
| D22 | Not on the internet until reviewed | **ABShare is not exposed publicly until the Phase 6 security gate passes** (§11). Earlier phases run on the LAN or behind a VPN only. The recommended test set-up is split-horizon DNS: the public name resolves internally to the real reverse proxy, which has a source-address allow-list for the LAN. That tests TLS, headers, cookies and passkeys (which need the real origin) end to end, with nothing public. Decided 2026-10-17. |
| D23 | Data-saver streaming | **A virtual constant-bitrate MP3 file, transcoded on demand in 60 s segments.** MPEG-2 Layer III at 24 kHz mono gives frames of exactly 576 samples (24 ms) and a whole number of bytes at every standard bitrate, with no padding (32 kb/s = 96 B, 40 = 120 B, 48 = 144 B). So the byte↔time mapping is exact, and the server can advertise a `Content-Length` for the whole book before any of it exists. A Range request is answered by transcoding only the 60 s segments it touches. Each segment is encoded with the bit reservoir off and a pre-roll that is a multiple of 2880 samples (aligned to the frame and to the 22.05/44.1 → 24 kHz resampling grid), then cut on exact frame boundaries. Segments therefore join with no gap or click. Playback stays a plain `<audio>` element with Range requests (D8): no HLS, no MSE, nothing changes for iOS background playback. The default is **32 kb/s (14.4 MB/h)**, about half to a quarter of typical originals; the Phase 0 listening test may choose 40 or 48. **The prototype was confirmed 2026-10-17** (scratch test, ffmpeg + libmp3lame): (1) every frame is 96 B; (2) segments made with an exact cut decode identically to a continuous encode; (3) the fast-seek variant is aligned to 0 samples with clean joins; (4) one 60 s segment from the middle of a 2 h AAC file took 0.19 s on one core (about 325× real time). **Phase 0(f) build, 2026-10-17:** the Rust spike (`tools/saver-spike`, jailed ffmpeg 7.1 in the hardened container) served a 2 h synthetic book as a 28.8 MB virtual file whose full download matched the advertised length exactly and decoded with no errors across all 121 joins. On the target host (i5-9500T, library over read-only NFS), an 85-hour test book (1.2 GB virtual) answered uncached seeks with the first audio in **0.72–0.80 s**. Segments there cost about 0.7 s each, most of it ffmpeg parsing the 85 h file's large `moov` index on every run. Phase 1 will encode several consecutive segments per ffmpeg run for prefetch and downloads, so the index is parsed once. Chosen over HLS: native HLS is missing in Firefox and patchy elsewhere, and an MSE player would add runtime code, a new attack surface and the iOS background-audio risks recorded in lyrion-anywhere. Decided 2026-10-17. |
| D24 | Supported deployments | **The Docker image, a systemd unit, or the Docker image inside an unprivileged LXC** (`nesting=1,keyctl=1`), with the library bind-mounted read-only at every layer. **The host kernel must have Landlock in its active LSM list.** Many distribution kernels compile it in but leave it out of the default list; the fix is the `lsm=landlock,…` boot option, documented in DEPLOY. `abshare doctor` checks Landlock and seccomp from inside the final container. **Phase 0(g) confirmed 2026-10-17** on a PVE 9 host (kernel 7.0) with `lsm=landlock,…`: inside Docker 29 inside an unprivileged LXC (`nesting=1,keyctl=1`), as uid 65534 with `--cap-drop ALL --read-only --security-opt no-new-privileges` and Docker's default seccomp profile, Landlock ABI 8 was available, file, exec and write confinement worked, and the process could stack its own seccomp filter. All 11 escape checks were refused. Decided 2026-10-17. |

---

## 2. Goals and non-goals

### Goals (v1)

- **Secure by default.** Default deny, invite-only accounts, phishing-resistant sign-in, mandatory second factor, revocable devices, sandboxed ffmpeg, a strict CSP, no outbound connections, and no path or internal address ever reaches a client.
- **Easy for listeners.** Open the invite link, create a passkey, and you're listening. Continue listening is one tap from Home. Nothing on screen needs explaining.
- **Your place follows you.** Stop on the phone and resume on the laptop to within a few seconds. Offline listening syncs back when the device reconnects.
- **Good audiobook playback:** chapters, skip back/forward (default 15 s back, 30 s forward), 0.5×–3× speed with pitch preserved, a sleep timer (time, or end of chapter), bookmarks with notes, lock-screen and headset controls, and AirPlay where the platform offers it.
- **Data saver:** stream or download any book at about 14 MB per hour on mobile data, with the same seeking, lock-screen and position behaviour as the original (D23).
- **Offline:** download a book into the app (original or data saver), or save the original file, if the user is allowed.
- **Light:** comfortable on a 2-vCPU, 4 GB VM shared with other services. Direct play costs almost no CPU, and transcoding is capped and queued.
- **Easy to run:** one binary or one container, one config file, and one command to create the first admin.

### Non-goals (v1)

- Native apps (the API is ready for them, §10).
- Uploading, editing or deleting library files (D3).
- Fetching metadata from Audible, Google Books or similar (D11).
- Removing DRM (`.aax`/`.aa`). Such files are listed as unsupported.
- Podcast RSS subscriptions; ebooks; social features; open sign-up; public share links.
- Chromecast. AirPlay works through Safari's own `<audio>` controls.
- Adaptive bitrate switching mid-stream. The listener chooses Original or Data saver (D23), or Automatic picks one when playback starts.

---

## 3. Architecture

```
  phone / tablet / desktop browser (PWA)
        │  HTTPS 443
        ▼
  reverse proxy (nginx / Caddy / Traefik)       TLS, HSTS, real client IP
        │  HTTP, loopback or a trusted LAN hop
        ▼
  ┌──────────────────────── abshare (one binary) ────────────────────────┐
  │  http: /v1 API · /v1/media (Range) · static PWA · security headers   │
  │  auth: accounts, passkeys, TOTP, sessions, devices, lockout, audit   │
  │  library: scanner → probe → core::assemble → catalogue               │
  │  jobs: playable copies · download transcodes · cover thumbnails      │
  │  core (no I/O): metadata rules, book assembly, chapters, progress,   │
  │                 validation, policy                                   │
  └──────┬────────────────────┬──────────────────────┬───────────────────┘
         │ read-only          │ read/write (0700)    │ spawns
         ▼                    ▼                      ▼
   library roots (:ro)   data dir: abshare.db,   abshare-jail ──► ffmpeg / ffprobe
   e.g. /srv/audiobooks  secret.key, covers/,    (Landlock + seccomp + rlimits:
                         copies/, transcodes/     reads only its inputs, writes
                                                  only its job dir, no network)
```

**Why this shape**
- One process and one trust boundary make the code easy to audit. The proxy only terminates TLS, and the app sets its own headers, including the CSP.
- Media is served by the same process that checks the session, so there is no separate file server to misconfigure.
- Everything that interprets untrusted bytes is either in `core` (pure, memory-safe, fuzzed) or inside ffmpeg running in a jailed child process (D19). Untrusted bytes include tags, chapter names, cue sheets, image data and ffprobe JSON.

### Workspace layout

| Path | What |
|---|---|
| `core/` | No I/O. Types, metadata normalisation, book assembly, chapter and position maths, progress merge, validators, ID derivation (given the key bytes). |
| `server/` | The `abshare` binary: HTTP, auth, DB, scanner, job runner, ffmpeg wrapper, CLI subcommands. |
| `jail/` | The `abshare-jail` helper (D19). Small enough to review in one sitting. Uses the `landlock` and `seccompiler` crates, with no `unsafe` of its own. |
| `fuzz/` | `cargo-fuzz` targets (D21). |
| `web/app/` | The SvelteKit PWA. |
| `deploy/` | Example nginx and Caddy configs, the systemd unit, docker-compose. |
| `docs/` | As in lyrion-anywhere: INSTALL, DEPLOY, SECURITY, PRIVACY, ARCHITECTURE, API, BUILDING, plus `plans/`, `security/` (threat model, ASVS map, reviews) and `release-notes/`. |
| `tools/demo/` | A generated demo library of synthetic audio with invented titles. It is used for tests, screenshots and benchmarks. Real library data never enters the repo. |

---

## 4. Library

### 4.1 Roots

- The config lists one or more libraries, each with a name and a root path, e.g. `Audiobooks = /srv/audiobooks` or `Kids = /srv/kids-audio`.
- Each user is granted all libraries or a chosen set.
- Paths never leave the server. Clients see library IDs and book IDs only.

### 4.2 Scanning

1. **Walk.** Every root is walked relative to an open directory handle.
   - **Symlinks are not followed** (on by default; an admin can allow links whose target stays inside the root, checked after canonicalising).
   - Special files, hidden entries (`.`-prefixed), names that are not valid UTF-8 and anything over a depth of 8 are skipped and reported.
   - The walk never crosses into another filesystem (`st_dev` must match the root's). Scans read only directory metadata until a file is new or changed. A root should be the narrowest directory that holds the books, because shared media stores often have tiered or spun-down storage next to the library.
   - A walk records `(relative path, size, mtime_ns)` per file. Only new or changed files go to step 2.
2. **Probe.** ffprobe runs through the jail (D19), one or two at a time at low priority, with a 60 s timeout.
   - The JSON output is capped at 4 MiB and parsed into strictly typed serde structs. Unknown keys are ignored. Wrong types, overflowing numbers, or strings over their caps make the probe fail.
   - A file that fails to probe is marked *unplayable*, with the reason shown to admins. A failed probe never stops the scan.
3. **Assemble** (`core`, pure). This turns directories of probed files into books (§4.3).
4. **Commit** in one transaction per library.
   - Books that disappear are marked *missing*, not deleted.
   - Users' progress and bookmarks for a missing book are kept for 180 days, so a move or remount doesn't lose anyone's place.

The audio extensions accepted are `.m4b .m4a .mp4 .mp3 .aac .ogg .oga .opus .flac .wav`. The probed container must agree with the extension, or the file is refused as unplayable. Everything else is ignored, except recognised sidecars (§4.4). `.aax`/`.aa` are listed as unsupported (DRM).

### 4.3 What makes a book

Real libraries come in every shape: a flat folder per title, `Author/Series/Title/`, a three-level-deep folder of numbered MP3 parts, and podcast episodes stored one per folder. The rules below are pure functions in `core`, tested against synthetic fixtures for each shape.

- **A loose audio file directly under a root** is a book on its own.
- **Inside a directory, the audio files it directly contains** are grouped like this:
  - Files with **different ASINs**, or different non-empty `album` tags, are **separate books**. This covers a flat folder of many books.
  - Files that look like **copies of the same book** are **alternates**. They have the same ASIN or the same normalised title, and durations within 1 %. Exactly one is chosen:
    1. it probes cleanly;
    2. it has a browser-safe codec (AAC-LC, MP3, Opus);
    3. it has the higher bitrate;
    4. it does not carry a sync-conflict style name (e.g. `….m4b-<digits>.<digits>-<tag>.m4b`).

    The others are listed to admins as *ignored duplicates*.
  - The remaining files are **parts of one book**, ordered by disc tag, then track tag, then natural filename order (`2` before `10`).
- **Nested directories** (`Book/CD1`, `Book/CD2`) belong to the parent book when the parent has no audio of its own and the children look like discs (names like `CD 1`, `Disc 2`, `Part 3`). Otherwise each leaf directory is assessed on its own.
### 4.3.1 Series and shows (automatic)

Store-sourced libraries use one tag, `series` (or `album` for podcast-style MP3s), for two different things:
- **book series**, for example eighteen 15–22 h novels;
- **episodic shows**, for example fourteen 20–45 minute episodes, each stored as its own "book".

Both are grouped by that series value. Each group is then classified automatically, with no admin work:

| Kind | Rule (first match wins) |
|---|---|
| **Show** | The genre includes "Podcast"; **or** at least ⅔ of the group's items are under 2 h **and** at least ⅔ of the titles match an episode pattern (below); **or** every item is under 2 h and the group has 3 or more items |
| **Book series** | Anything else with 2 or more items (a series may contain a short story or novella; one short item doesn't make it a show) |
| (no group) | A single item with no series is a standalone book |

**Shows** appear as one tile in the library. They open to an episode list with "Continue" pointing at the first unfinished episode. Progress is tracked per episode, and "Continue listening" on Home shows the show, not 14 separate episodes.

**Ordering within a group** (pure function in `core`, fixture-tested):
1. **Shows:** order by **season, then episode number parsed from the title**. Store `part` tags must not be trusted for shows: they are often a sentinel (`999999000`, `0`) and sometimes plausible-looking numbers in the wrong order (release or purchase order).
   - Recognised patterns (case-insensitive): `Ep. 7:`, `Ep: 7:`, `Ep 7:`, `Episode 7`, `E7 -`, `S2E7`, `Season 2 - … - E7`, `Part 7`, and `07-55` style (part of total).
   - Then the release date, then natural title order.
   - **Extras** (titles matching trailer, announcement, update, bonus, aftershow, preview, "presents") go into an "Extras" section after the episodes, in date order, so a trailer never becomes "next episode".
   - If titles show more than one season ("Season 1 …", "Season 2 …"), the episode list is sectioned by season. One show tag can span seasons or spin-offs; the show/season split comes from the titles.
2. **Book series:** order by `part` when it is a sane number (> 0 and < 100000; fractional allowed, e.g. "1.5"). Then by a "Book N" number parsed from `album`/`title`. Then by release year, then title. `part = 0` (prequels and companions) sorts first. A trailing bracketed qualifier in the series name (e.g. "[Chronological Order]") is kept as a subtitle on the series page, not in the series name used for grouping.

The listing shows the series position ("Book 3") only when it came from a sane `part` or a parsed "Book N", never from a sentinel.

Admins can override the kind, the order or the grouping of any group. Overrides are stored by stable ID and survive rescans, but are never needed for the common cases above.

### 4.4 Metadata

`core::metadata` normalises tags into a `BookMeta`. Every field is optional and every string is untrusted.

| Field | Sources, in order |
|---|---|
| Title | `title`; `album` with suffixes like " (Unabridged)" and ", Book N" removed; folder name with a trailing `[ID]` removed |
| Subtitle | `subtitle` |
| Authors | `album_artist`, then `artist`. Split on `, ` and `;` only: store tags join people with ", ", and splitting on "&" or "and" would break names. Keep "Jr."/"Sr." suffixes with the preceding name. Strip a role suffix such as " - adaptation" or " - translator" into a role field. Drop empty entries, since lists may be truncated with a trailing ", ". |
| Narrators | `composer` (Audible convention), `narrator`, `performer`; a sidecar `reader.txt` or a "Read by …" line in a `.txt`; a "(Narrated by X)" suffix in the title, which is moved out of the title. Split as for authors. Full-cast productions can list ten or more names, so the UI shows the first three plus "and N others". |
| Series + position | `series` + `part`, then `mvnm`/`mvin`, then "…, Book N" parsed from `album`/`title`. `part` values ≥ 100000 are sentinels and treated as missing. `0` means a prequel or companion (sorts first, no number shown). A position may be fractional ("1.5"). Shows ignore `part` (§4.3.1). |
| Description | `comment`, `description`, `lyrics`/`lyrics-eng`, `desc.txt`. HTML is converted to **plain text** on the server. |
| Genres | `genre`, split on commas |
| Year, publisher, language | `date` (year only), `publisher`, `language` |
| ASIN / ISBN | `audible_asin`, `asin`, `isbn`, then a trailing `[B0XXXXXXXX]` or `[digits]` in the folder name; validated by pattern |
| Cover | the embedded attached picture, then `cover.*` / `folder.*`, then any single image in the folder |

Cleaning rules (all in `core`, all fuzzed):
- Text is NFC-normalised.
- Control, bidi-override and zero-width characters are removed.
- Whitespace is collapsed.
- Length is capped per field (titles 512, descriptions 20 000).
- HTML-to-text keeps paragraph breaks only and decodes entities. It is a bounded tokenizer of ours, never a browser parser.
- Anything that looks like a filesystem path or a LAN address is dropped from free text.
- Sidecar text files over 64 KiB are ignored.
- Clients still render everything as **text only** (§9.3).

Admins can override any field in the UI. Overrides are stored in the DB by book ID, validated by the same rules, and survive rescans.

### 4.5 Chapters

- **Embedded chapters** (MP4 `chpl`/QuickTime chapters, ID3 `CHAP`) are used when present and sane. Sane means monotonic and inside the duration; small overlaps and gaps are clamped. At most 2000 chapters are kept.
- **Multi-part books without chapters** get one chapter per part, named from the part's `title` tag or a cleaned filename. When the parts have their own chapters, those are offset onto the book timeline.
- **`.cue` sidecars** are used only when nothing better exists. The parser is bounded and fuzzed, and its `FILE` lines are never used as paths.
- **A long single file with no chapters** gets no invented chapters. The player shows the whole book as one span.

### 4.6 Covers

- The source image (embedded or sidecar) is extracted with jailed ffmpeg (`-map 0:v:0 -frames:v 1`, copied, capped at 20 MB).
- It is then **decoded and re-encoded in Rust** with the `image` crate: decoder limits on width, height and allocation, and only JPEG and PNG accepted. The outputs are WebP or JPEG at 128, 320, 640 and 1280 px. Re-encoding strips all metadata and neutralises malformed-image tricks. A decode panic is caught by the job runner and the book gets no cover.
- Thumbnails are keyed by book ID + source fingerprint and served immutable with a hashed URL.

---

## 5. Server

### 5.1 HTTP surface

- It listens on `127.0.0.1:8743` by default (set `listen` in config, e.g. `0.0.0.0:8743` behind a proxy on another host).
- It speaks plain HTTP. The proxy terminates TLS.
- As in lyrion-anywhere, a request counts as TLS only when a **trusted proxy** sets `X-Forwarded-Proto: https`. Non-TLS requests from anything other than loopback get `403 TLS required`.
- `X-Forwarded-For` is read from the right, skipping trusted hops, and believed only from `trusted_proxies`. An optional `client_ip_header` (e.g. `CF-Connecting-IP`) is honoured only from a trusted proxy.
- The `Host` header must match `public_origin`. Otherwise the answer is 421. This defeats DNS rebinding and host-header tricks.
- Limits:
  - 1024 connections overall and 64 per client address;
  - 15 s for a request head; 15 s for a request body (but not for media responses);
  - bodies of 64 KiB at most (4 KiB for auth routes);
  - per-user and per-device rate windows on every write route, and per-address windows on every unauthenticated route.
- Only the methods each route declares are accepted (405 otherwise). `TRACE` and `OPTIONS` are refused, so no CORS is ever granted.
- `/healthz` returns `ok` with no detail. There is no version string in headers or unauthenticated responses.

### 5.2 Media (`GET /v1/media/{book_id}`)

- Authentication is by the media cookie (D9). The book must be in a library the user may access, or the answer is 404. 403 is never used, so a book's existence isn't revealed.
- The server serves the original file for direct play, or the playable copy (D7). Single-range `Range` is supported, with `Content-Length`, `Accept-Ranges: bytes` and `ETag` = an HMAC of the file fingerprint. Multi-range requests get the whole file (200), and unsatisfiable ranges get 416.
- Before opening, the path from the catalogue is re-checked against its root. It is opened with `O_NOFOLLOW`, and `fstat` must match the catalogued size and inode. On a mismatch the server refuses and queues a rescan.
- Headers:
  - `Cache-Control: private, no-store`;
  - `Content-Type` from the probed container, never from the extension;
  - `Content-Disposition: inline`;
  - `X-Content-Type-Options: nosniff`;
  - a sandboxing `Content-Security-Policy: default-src 'none'; sandbox`.
- **Data saver: `GET /v1/media/{book_id}?q=saver`** (D23).
  - The response is a virtual CBR MP3 of `frames × frame_bytes` bytes, where `frames = ceil((duration_samples + 1105) / 576)` at 24 kHz (1105 is the encoder delay). It has no ID3 or Xing header, so byte 0 is the first frame and the timeline starts at 0.
  - The ETag is an HMAC of (source fingerprint, profile version).
  - Single-range requests map to segment indices: segment = 2500 frames = 60 s at 32 kb/s = 240 000 bytes. The server streams the requested byte slice from the cached segments, starting each missing one as a job (§5.4) and sending its bytes as soon as it is written. A Range inside one segment waits at most about 0.3 s for that segment to be made.
  - **Prefetch:** after serving segment *n*, segments *n+1* and *n+2* are queued at low priority, so playback never waits at a boundary.
  - **The time base is shared with the original:** the client converts its position in milliseconds to and from either source, so switching quality mid-book keeps your place (the 46 ms encoder delay is applied as a fixed offset).
- A book whose playable copy isn't built yet answers `409 {"state":"preparing","progress":0.42}`. The app shows "Preparing this book…", and building is moved to the front of the queue (at most once per user per minute).

### 5.3 Downloads

- **In the app (offline listening)**: `GET /v1/downloads/{book_id}?profile=original|saver`. Range requests are supported, so chunked, resumable fetches into IndexedDB work (§8). `saver` is the same virtual MP3 as data-saver streaming (D23). A download walks its segments in order at the lowest job priority, so a whole-book data-saver download costs one pass of transcoding (about 3 minutes for an 85 h book), and its segments are then cached for streaming as well. Allowed by the `offline` permission (on by default).
- **Save file** (`GET /v1/files/{book_id}`): the original file, or a ZIP (stored, not compressed) of the parts with a cleaned filename. `Content-Disposition: attachment`. This needs the `save_files` permission (off by default). It is audited and limited to 3 at a time per user, with an optional daily cap per user.
- Download routes also accept a short-lived (5 min) single-use ticket from `POST /v1/downloads/{id}/ticket`. A browser's own download manager can't send cookies scoped to `/v1/media`, so the ticket covers that case.
  - The ticket goes in a POSTed form field, not the URL, so it never appears in logs.
  - Tickets are hashed at rest and bound to user, device and book.

### 5.4 ffmpeg wrapper, jail and job runner

This carries over lyrion-anywhere's `transcode.rs` rules unchanged and adds the jail (D19):

- **The jail.** The process chain is `abshare` → `abshare-jail --read <in>… --write <jobdir> --limits … -- ffmpeg <args>`.
  - The jail applies Landlock, seccomp, no-new-privs and rlimits, closes every inherited file descriptor except 0–2, and replaces itself with ffmpeg.
  - A test proves that a jailed process can't open `/etc/passwd`, the DB, the secret or a sibling book, can't create a TCP socket, and can't write outside its job directory.
- **Arguments.**
  - A fixed argument vector, no shell, `env_clear()`, and `-nostdin -hide_banner -loglevel error`.
  - **The input demuxer is forced** from a fixed table keyed by probed format (`-f mov`, `-f mp3`, `-f ogg`, `-f flac`, `-f wav`). HLS, concat, ffconcat, image2, lavfi and tee can never be selected, and the minimal build doesn't contain most of them anyway.
  - `-protocol_whitelist file`, `-enable_drefs 0` (mov), `-map 0:a:0` (or the chosen streams), `-map_metadata -1`, then sanitised tags and server-generated chapters re-added from an ffmetadata file the server writes itself, with escaped values.
  - Multi-part books are joined with the **concat filter over explicit `-i` inputs**, never the concat demuxer or protocol.
  - `-threads 1`, `-fs <cap>` (input size × 2 + 64 MiB), a wall-clock timeout scaled to duration, `kill_on_drop(true)`, `nice` 10, and stderr captured up to 64 KiB for admin diagnostics. Diagnostics are never shown to listeners.
- **Output.** Output goes to a temporary name in the job directory, then it is probed again (jailed), `fsync`ed and `rename`d into place. A crashed or bad job leaves nothing that could be served.
- **Data-saver segments** (D23). Each segment is a jailed ffmpeg run:
  - `-ss <t − 3 s> -copyts` on the input (fast seek), then `aresample=24000,asettb=1/24000,atrim=start=<a>:end=<b>`, then libmp3lame CBR with `-reservoir 0`, no Xing and no ID3.
  - The window starts at `a = segment start − 25 920 samples` (a pre-roll of 45 frames, a multiple of 2880 so that it aligns with both the frame and the resampling grid). The server keeps exactly frames 45 to 45 + 2500.
  - Segment 0 is decoded from the start with no seek, because seeking to 0 shifts it by the encoder delay; this was found in the prototype.
  - Output frames are checked before caching: there must be exactly the expected number, every header must be the expected one, and the byte count must be exact. A segment that fails is never served; the request gets a 503 and the event is logged.
  - Limits: segment jobs are deduplicated by (book, profile, index). Each user may have at most 4 segment jobs in flight, plus prefetch. Overall they share the job runner's concurrency (§5.4), with a separate fast lane of 1 so a long playable-copy job never blocks streaming.
- **Queue.** Jobs run in a **persistent queue** (SQLite table):
  - kinds: playable copy, data-saver segment, cover extraction;
  - priority: a listener waiting on a segment > prefetch > downloads > background preparation;
  - 1 concurrent ffmpeg job by default (configurable, but capped at the number of CPU cores);
  - jobs are idempotent and keyed by book + profile + profile version (`PROFILE = "v1"`, bumped whenever settings change).
- **Cache.** Size-capped LRU (default 10 GiB). Playable copies are **pinned**; data-saver segments are evicted first. At 14.4 MB per hour, 10 GiB holds about 700 hours of data-saver audio.

### 5.5 Accounts, sessions and devices

As lyrion-anywhere, unless stated otherwise.

**Invites**
- An admin (after step-up) creates an invite: an optional display name, the libraries, permissions and role (listener by default; inviting an admin needs a second confirmation), and an expiry (default 72 h, at most 7 days).
- The link is `https://host/#/invite/<token>`, with a 256-bit token stored hashed. It works once. The fragment keeps the token out of server and proxy logs, and the app posts it in a body.
- Admins can list and revoke unused invites.

**Enrolment**
- The invitee chooses a username (`[a-z0-9._-]{3,32}`, after confusables folding), then **creates a passkey** (recommended, shown first) or a **password + TOTP**.
- Passwords are 12–1024 characters. They are checked against a bundled breached-password list (top 1M as a compact filter) and refused if they contain the username.
- Then 10 recovery codes are shown once and stored hashed.

**Passwords**
- argon2id (`argon2` crate). Parameters are m = 64 MiB, t = 3, p = 1, stored in the PHC string, so they can be upgraded on next sign-in.
- At most 4 hashes run at once, in `spawn_blocking`. Others wait up to 2 s, then get a generic "busy".
- An unknown username is checked against a dummy hash, so timing gives nothing away.

**TOTP**
- RFC 6238: SHA-1, 30 s steps, 6 digits, ±1 step.
- Single use: the last used step is stored.
- Constant-time comparison.
- The secret is 20 bytes, encrypted at rest (XChaCha20-Poly1305) with a key derived from the server secret, so a copy of the DB alone doesn't yield TOTP seeds.
- Wrong codes count towards the same lockouts as wrong passwords.

**Passkeys**
- `webauthn-rs` (the safe `Passkey` API) with **user verification required**, an RP ID from `public_origin`, and the origin checked exactly.
- Several passkeys per user, each named and revocable. Discoverable credentials allow username-less sign-in.
- Challenges are single use, last 2 minutes, and are bound to the session that requested them.

**Recovery**
- A recovery code replaces the second factor once, then forces re-enrolment of a factor.
- There is no email recovery: the server has no outbound connections. An admin can reset a user's factors (step-up, audited); the user gets a new single-use enrolment link that the admin passes on out of band.

**Lockout** (failures count before the hash is checked)
- Per client address (IPv6 per /64): 5 failures in 15 min triggers a hard lock that starts at 15 min, doubles each time and caps at 24 h.
- Per username: a soft lock. Addresses that have failed are refused, and other addresses share 2 attempts per minute, so a stranger can't lock a user out.
- Errors are always generic. Every failure and lock is audited.

**Tokens**
- Access token: 256-bit, 15 min, held in memory on both sides, hashed in the server map.
- Refresh token: 90 days by default (admin can set 1–365), rotated on every use, reuse revokes the device, 60 s grace, renewals capped at 30 a minute per device. Stored in the `abshare_refresh` cookie (`HttpOnly; Secure; SameSite=Strict; Path=/v1/auth`).
- Media cookie: D9.
- An **absolute session lifetime** (default 90 days) applies even with continuous refresh; after it, the user signs in again.
- Browser tabs take turns refreshing (`navigator.locks`).
- Changing a password or factor revokes every other device of that user.

**CSRF**
- Cookie-reading routes (`/v1/auth/*`) require `X-ABShare-Client: 1` and an exact `Origin` match. No CORS preflight is ever approved.
- Everything else uses bearer tokens, and media GETs change nothing.

**Devices**
- Each sign-in is a device. Limits: 20 per user (the least recently seen is evicted) and 10 new per hour.
- Device names are cleaned.
- Each device's last IP and User-Agent are visible only to the user and admins, and are erased when the device is revoked.
- "Your devices" lets users sign any device out. Limits on that route are keyed per device, so a stolen token can't starve the owner's sign-out (lyrion-anywhere review finding).

**Step-up**
- Sensitive actions need a fresh factor within the last 10 minutes: changing password or factors, viewing recovery codes, and every write on the admin pages.

**Roles and permissions**
- Roles are `admin` and `listener`.
- Permissions: `offline` (default on), `save_files` (default off), libraries (all or a set).
- The last admin can't be demoted or deleted.

**Audit log**
- Records sign-ins (success and failure), locks, enrolment, factor changes, invites, device revocations, permission and setting changes, saved files, jail violations (seccomp kills) and admin actions.
- Rows are append-only from the app's point of view and hash-chained (each row includes the HMAC of the previous one), so tampering is detectable.
- Keeps one year, at most 100 000 rows, pruned in batches with the chain re-anchored.
- Admins see it in the UI. It can also be mirrored to stdout as JSON for an external collector.

### 5.6 Progress, bookmarks and history

**Progress**
- One row per user per book: `position_ms`, `duration_ms`, `finished`, `updated_at` (client event time), `received_at`, `device_id` and `speed`.
- Clients send `PUT /v1/progress/{book}` with an **idempotency key** and the client event time:
  - every 15 s while playing;
  - on pause, seek, chapter change and `visibilitychange`→hidden (`fetch` with `keepalive`);
  - from the offline queue on reconnect.
- Merge (in `core`, property-tested):
  - Accept if `event_time > stored.updated_at`.
  - Refuse event times more than 5 min in the future or older than 90 days.
  - Clamp positions to `[0, duration]`.
  - A duplicate idempotency key is a no-op that returns the stored state.
  - `finished` is set when position ≥ duration − 30 s or the user marks it; it is cleared by an explicit "mark unfinished" or by starting again from the beginning.
  - Writes are limited to 12 a minute per device per book.
- **"Resume where you left off?"** When the server's position came from *another device* more recently than this device's local position, the app offers both: "Continue from 4:12:09 (on Laptop, 2 h ago)" or "Stay at 3:58:40". It never jumps silently by more than 30 s.

**Bookmarks**
- Position, an optional note of up to 2000 characters, and created/updated times.
- At most 1000 per user per book; synced like progress.

**Listening history**
- Sessions of start, end and seconds heard, per user. They feed "Continue listening", "Recently finished" and a per-user listening-time stat.
- Kept 13 months. Users can view, clear or turn it off.
- Admins can't read other users' history. Only aggregate "last active" times are shown.

### 5.7 Configuration (`abshare.toml`)

```toml
listen          = "127.0.0.1:8743"
data_dir        = "/var/lib/abshare"
public_origin   = "https://books.example.com"   # RP ID, allowed Origin and Host derive from this
trusted_proxies = ["127.0.0.1"]
# client_ip_header = "CF-Connecting-IP"

[ffmpeg]
ffmpeg  = "/usr/lib/abshare/ffmpeg"
ffprobe = "/usr/lib/abshare/ffprobe"
jail    = "/usr/lib/abshare/abshare-jail"
jobs    = 1
# allow_unsandboxed_ffmpeg = false   # D19; never set this on an internet-facing install

[cache]
max_gib = 10

[[library]]
name = "Audiobooks"
path = "/srv/audiobooks"
```

At start-up the server refuses to run when:
- the config or data dir is group- or world-writable;
- the secret is readable by others;
- a library root is writable by the server's own user (D3; overridable for development only, with a warning).

Admins change the rest in the UI and the database stores it: scan interval, the 2FA policy, default permissions, invite expiry and session lifetimes.

### 5.8 CLI

- `abshare serve [--config …]`
- `abshare admin create-user NAME --role admin` (prints a single-use enrolment link)
- `abshare admin reset-factors NAME`, `list-users`, `list-devices`, `revoke-device`, `revoke-all` (emergency: signs every device out)
- `abshare scan [--library NAME]`, `abshare check-config`
- `abshare doctor`: checks the ffmpeg version, decoders and absence of network protocols, Landlock and seccomp availability (with a live jail self-test), data dir permissions, a read-only library and the proxy headers.

Admin subcommands take the DB lock and refuse to run while the server is running, unless `--online` is given. With `--online` they use a Unix socket (mode 0600) that only the server's own user can open.

---

## 6. API (v1)

All JSON. `Authorization: Bearer <access>` unless noted. `deny_unknown_fields` on every request body. IDs are opaque base32 strings, validated by length and alphabet before lookup.

| Endpoint | Purpose | Auth |
|---|---|---|
| `POST /v1/auth/invite/inspect` | Check an invite token is valid (returns display name and expiry) | none (rate-limited) |
| `POST /v1/auth/enrol/*` | Username, password, TOTP and passkey registration, recovery codes | invite token |
| `POST /v1/auth/login/password` | Step 1: username + password, returns a challenge for the second factor | none |
| `POST /v1/auth/login/totp` · `/recovery` | Step 2 | challenge |
| `POST /v1/auth/login/passkey/{start,finish}` | Passkey sign-in | none |
| `POST /v1/auth/refresh` · `/logout` | Rotate the refresh token, issue access and media cookie; sign out (`Clear-Site-Data: "cache"`) | refresh cookie + CSRF header |
| `POST /v1/auth/step-up/*` | Re-verify a factor for sensitive actions | bearer |
| `GET /v1/me` · `PATCH /v1/me` | Profile, preferences (skip intervals, default speed, theme) | bearer |
| `GET/POST/DELETE /v1/me/{passkeys,totp,devices,recovery}` | Manage one's own factors and devices | bearer (+ step-up for changes) |
| `GET /v1/libraries` | Libraries the user can see | bearer |
| `GET /v1/books?library=&q=&author=&narrator=&series=&genre=&status=&sort=&cursor=` | Browse and search (FTS5 with parameter binding only, prefix match, diacritics folded) | bearer |
| `GET /v1/books/{id}` | Detail: metadata, chapters, duration, play state, the caller's progress and bookmarks | bearer |
| `GET /v1/authors/{id}` · `/series/{id}` · `/collections/{id}` | Grouped views | bearer |
| `GET /v1/covers/{id}/{size}.{hash}.webp` | Cover thumbnail (immutable) | media cookie |
| `GET /v1/media/{id}` | Stream (Range) | media cookie |
| `GET /v1/downloads/{id}?profile=` · `POST /v1/downloads/{id}/ticket` | Offline download (Range) and its ticket | media cookie or ticket |
| `POST /v1/files/{id}` | Save the original file | ticket, `save_files` |
| `PUT /v1/progress/{id}` · `GET /v1/progress?since=` | Report progress; fetch changes since a cursor | bearer |
| `POST /v1/progress/{id}/finished` · `DELETE …/finished` | Mark finished or unfinished | bearer |
| `GET/POST/PATCH/DELETE /v1/bookmarks…` | Bookmarks | bearer |
| `GET /v1/home` | Continue listening, recently added, series in progress (downloads are merged in on the client) | bearer |
| `GET/DELETE /v1/history` | Listening history | bearer |
| `/v1/admin/*` | Users, invites, permissions, devices, libraries and scans, jobs, cache, metadata overrides, settings, audit | bearer, admin role, step-up for writes |

Rules:
- Errors are `{"error": "<code>", "message": "<text>"}` with stable codes. Internal errors return a generic message plus a correlation ID that is logged server-side. Stack traces, paths and SQL never reach a client.
- Only the server's own JSON 401 means "signed out". The app never signs out because of a proxy's 502/504.
- Pagination uses opaque cursors with a page limit of 100 at most. No endpoint fans out into unbounded work per request.
- The PWA uses only this documented API.
- A contract fixture lists every request the web app can make. CI runs each one through the real router, so the app can't depend on an undocumented route.

---

## 7. Web app

### 7.1 Stack

- SvelteKit 2, Svelte 5 runes, TypeScript and Vite, built with `adapter-static`, a hash router and relative paths, so it works under any base path.
- Exact-pinned dev dependencies only, installed with `npm ci` and lockfile integrity; **no runtime dependencies**.
- `svelte-check --fail-on-warnings`, Vitest + Testing Library, and Node tests for policy files.
- Reused from lyrion-anywhere:
  - the boot-script externaliser, with a build that fails on any inline script, `on*=` attribute or `javascript:` URL;
  - the source scan that bans: Svelte's raw-HTML tag, the `innerHTML`/`outerHTML` setters, `insertAdjacentHTML`, the document `write`/`writeln` methods, `eval`, the `Function` constructor, string arguments to `setTimeout`/`setInterval`, `createContextualFragment`, `srcdoc`, `DOMParser`, and `on*` attributes set with `setAttribute`;
  - the theme system: Material 3 tonal palettes from one accent in OKLCH, light/dark/auto and high contrast, with WCAG contrast tested for every combination.
- **Trusted Types** are enforced where supported (`require-trusted-types-for 'script'` with no policies defined), so any DOM-XSS sink use fails loudly in Chromium.

### 7.2 Screens

The UX rules come from lyrion-anywhere's declutter plan:
- one status slot;
- at most two buttons plus ⋮;
- destructive actions only inside ⋮;
- one way in to each feature;
- one word per concept;
- 44 px touch targets;
- WCAG 2.2 AA.

**Screens**
- **Home:** Continue listening (big cards with progress and time left), Series in progress (next book), Recently added, Downloaded.
- **Library:** grid or list; search; filter by author, narrator, series, genre and status (not started, in progress, finished); sort by title, author, recently added or recently played. Collections of episodes appear as one tile.
- **Book:** cover, title, series ("Book 2 of …" linking to the series), authors and narrators linking to their pages, duration, time left, description (text), chapters. Actions are Play/Resume and Download, with ⋮ for mark finished, start again, bookmarks and save file.
- **Player** (full screen, opened from the mini player):
  - cover and chapter title;
  - a **chapter-relative scrubber** with whole-book progress underneath;
  - −15 s, play/pause, +30 s, previous and next chapter;
  - speed, sleep timer, chapters and bookmarks.
  - Gestures always have button equivalents.
- **Mini player:** fixed at the bottom on every screen.
- **Downloads:** per-book state, sizes, storage used against quota, persistence status, and remove buttons.
- **Settings** (an index of rows, each with a one-line summary): Playback (skip intervals, default speed, auto-rewind, streaming quality: Automatic / Original / Data saver), Downloads (default quality, Wi-Fi only where detectable), Appearance, Account (name, passkeys, two-step, recovery codes), Devices, History, About.
- **Admin:** Users and invites, Libraries and scans (problems list: unplayable files, ignored duplicates, missing books), Jobs and cache, Audit log, Security status (2FA coverage, sandbox state, warnings), Server settings.

### 7.3 Playback details

- **One `<audio>` element** is created at start-up and reused for every book; its `src` is changed. It is never recreated, because iOS ties background permission to the element.
- **Media Session:**
  - metadata (book title, chapter as the artist line, author, cover at several sizes, as blob URLs);
  - actions: play, pause, `seekbackward`/`seekforward` (with the user's intervals), `previoustrack`/`nexttrack` as previous and next chapter, and `seekto`;
  - `setPositionState` kept current, including the playback rate.
  - Only actions that work are registered.
- **Streaming quality** (D23):
  - **Automatic** picks Data saver when the Network Information API reports a cellular or `saveData` connection (Chromium on Android), and Original otherwise. Browsers without the API (Safari, Firefox) get Original unless the user chooses.
  - The current quality is shown in the player's ⋮ menu with a one-tap switch. Switching swaps the `src` at the same millisecond position.
  - A downloaded book always plays from the device, whatever this setting says.
- **Speed:** `playbackRate` with `preservesPitch = true`. It is remembered per book, with a user default.
- **Sleep timer:**
  - durations of 5, 10, 15, 30, 45 and 60 minutes, or end of chapter;
  - fades out over the last 10 s;
  - it runs from `timeupdate` events, not `setTimeout`, so it survives background throttling.
- **Auto-rewind:** resuming after a pause rewinds a few seconds, scaled by how long it was paused (default 0 s under 1 minute, 5 s after 10 minutes).
- **Errors:**
  - `play()` NotAllowedError means "tap to play".
  - An AbortError after a source change is retried once.
  - A network stall gets backoff retries from the current position.
  - A 401 from the media cookie triggers a refresh, then a retry.
- **Reload and kill:** on load the app restores the last book and position. With an explicit user tap, it offers "Resume playing" rather than autoplaying.
- **iOS:**
  - volume is read-only (feature-detected), so the volume control is hidden;
  - offline storage is reliable only as an installed home-screen app, and the Downloads screen says so;
  - iPadOS reports itself as macOS, so install detection allows for that;
  - the minimum supported version is iOS 17.1.

### 7.4 Service worker

lyrion-anywhere's policy:
- **What is cached:** the app shell only, in a versioned cache.
- **Strategies:** cache-first for `_app/immutable/`; network-first for index, the manifest and icons.
- **Bypassed:** `/v1/*`, media, covers, query strings, non-GET requests and other origins.
- **Stored:** only `200`, `type === 'basic'`, non-`no-store` responses.
- **Updates:** a new version waits, and the app shows "Update available, reload" instead of `skipWaiting` mid-listen.

---

## 8. Offline listening

### 8.1 What

- One book per queue entry, with several queued.
- Quality is **Original** or **Data saver** (32 kb/s mono MP3, D23), chosen per download with a default in Settings. A data-saver download is typically a quarter to a half the size of the original: an 85 h book is about 1.2 GB instead of 2.4 GB.
- A book's size and the free quota are shown before downloading. The quota is checked with `navigator.storage.estimate()`, and `navigator.storage.persist()` is requested.

### 8.2 Storage and fetching

- IndexedDB, one database per user ID.
- Books are stored as 4 MiB Blob chunks fetched with Range requests, so downloads resume after interruption or a page kill. A manifest records chunks, ETag and profile.
- An ETag change is noticed on resume. The partial download is discarded and restarted.
- Covers and book JSON are stored with the book, so the book page and player work fully offline.
- Signing out offers "Keep downloads on this device" or "Remove". Another user signing in on the same browser never sees the first user's downloads. A device revoked remotely wipes its downloads the next time the app runs and receives the revocation.

### 8.3 Playback offline (Phase 0 spike decides)

- **Option A:** `URL.createObjectURL(new Blob(chunks, {type}))` as the `<audio>` src. This is simple. The risk is memory and seeking behaviour with 1–2.5 GB books on iOS.
- **Option B:** a service-worker route `/offline/{book}` that answers Range requests from IndexedDB chunks. This scales to any size, but Safari's media-through-SW Range handling has to be proved.

The spike tests both with a 2.5 GB, 85 h file on iOS 17/18 (installed PWA, screen locked) and on Android Chrome. The losing option is dropped.

### 8.4 Sync-back

- Progress, bookmarks and finished marks made offline go into a per-user IndexedDB outbox with idempotency keys. They are flushed on reconnect, oldest first, and each is removed only on the server's acknowledgement.
- The server's merge rules (§5.6) settle conflicts.

---

## 9. Security

### 9.1 Principles

- **Default deny.** Every route has an explicit auth requirement declared in one routing table. A test fails if any route lacks one. Every body has a strict schema, and every ID is looked up and access-checked before use.
- **No client-supplied paths, ever.** A client names a book ID, and the server resolves it to a path it found itself during a scan.
- **Untrusted metadata.** Tags, chapter names, cue sheets, sidecar text and images all come from files and may be hostile. They are parsed in `core` or re-encoded (covers) and always rendered as text.
- **Assume ffmpeg can be compromised.** It runs jailed (D19), so a media-parser exploit gains nothing.
- **Least privilege at run time.** A dedicated user, a read-only library mount, a 0700 data dir, a 0600 secret, no capabilities, a read-only root filesystem, and no outbound network.
- **Security-critical code in one memory-safe component**, with negative tests per surface (§13).
- **Fail closed.** A missing config value, an unavailable sandbox, a DB error during an auth check, or an unknown enum value is a refusal, never an allow.

### 9.2 Threat model

| Threat | Mitigation |
|---|---|
| Internet scanners and credential stuffing | Invite-only; generic errors; IP and username lockout (§5.5); breached-password list; 2FA required; passkeys preferred; nothing useful without auth beyond `/healthz` and static app files; no version disclosure. |
| Phishing for credentials | Passkeys are origin-bound. TOTP is the fallback; the UI steers everyone to passkeys. |
| Stolen or lost device | Revoke from "Your devices" or by an admin, or `revoke-all`. Refresh rotation detects reuse. Access tokens last 15 min. Media cookies are revoked with the device. Absolute session lifetime. |
| Stolen DB backup | argon2id hashes; token, invite, recovery and ticket hashes only; TOTP seeds encrypted with a key from the separately stored server secret. |
| Path traversal or arbitrary file read | IDs only; a path is resolved from the scan catalogue and re-checked against the root before open; `O_NOFOLLOW`; inode and size match; symlinks not followed by default. |
| Malicious media file attacking ffmpeg or ffprobe | Minimal ffmpeg build; forced demuxers; `-protocol_whitelist file`; **Landlock + seccomp + rlimits jail** (D19); no network; the library is admin-curated. |
| Malicious image attacking a decoder | The `image` crate (memory-safe) with decode limits; only JPEG and PNG; the output is re-encoded; never served raw. |
| XSS via tags or descriptions | Server-side text conversion; client text-only rendering; banned-API source scan; strict CSP; Trusted Types. |
| Host header, DNS rebinding | `Host` must match `public_origin`; no CORS. |
| Book existence probing across libraries | 404 for both "doesn't exist" and "not allowed"; IDs are 128-bit HMACs, so they can't be enumerated. |
| Download abuse or bandwidth exhaustion | `save_files` off by default; 3 concurrent per user; daily caps; single-use tickets; audit log. |
| Progress tampering or replay | Idempotency keys; bounded event times; clamped positions; per-device rate limits. |
| Resource exhaustion | Connection, header and body caps; job queue concurrency of 1; no per-request ffmpeg; capped JSON; LRU cache cap; limiter tables with eviction; SQLite writes off the async workers. |
| Malicious or compromised admin account | Step-up on every admin write; hash-chained audit; admins can't read other users' history or factors; inviting an admin needs a second confirmation. |
| Misconfigured proxy (spoofed client IP, double CSP, plain HTTP) | Trusted-proxy list; read from the right; `doctor` checks; the CI deploy test checks a single CSP, TLS redirect and spoofed XFF; `403 TLS required`. |
| Cross-account leakage on a shared browser | Per-user IndexedDB; covers fetched through per-user URLs; `Clear-Site-Data: "cache"` on sign-out; no shared caches. |
| Secrets leaking through logs or crash output | Redacted `Debug` for every secret type (tested); no tokens in URLs; panics never print request data; core dumps disabled. |
| Supply chain | Minimal dependencies (each new crate justified in its PR); cargo-deny (licences, advisories, bans, sources); `cargo audit` nightly; Docker base images pinned by digest; CI actions pinned by SHA; no runtime JS dependencies; ffmpeg built from a pinned, checksummed source tarball; reproducible release builds with published SHA-256 checksums. |

### 9.3 Hardening

**Headers on every response, refusals included**
- `Content-Security-Policy: default-src 'none'; script-src 'self'; worker-src 'self'; manifest-src 'self'; connect-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; style-src 'self'; style-src-attr 'unsafe-inline'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; require-trusted-types-for 'script'`
- `Permissions-Policy` turning everything off except `autoplay=(self)`, `publickey-credentials-get=(self)` and `publickey-credentials-create=(self)`.
- `X-Content-Type-Options: nosniff`, `Referrer-Policy: no-referrer`, `Cross-Origin-Opener-Policy: same-origin`, `Cross-Origin-Resource-Policy: same-origin`, `X-Frame-Options: DENY`.
- `/v1/*` gets `Cache-Control: no-store`; covers get `private, max-age=31536000, immutable`.
- HSTS (with `includeSubDomains`, a long max-age, and preload if the domain allows) comes from the proxy. The proxy must not add a CSP.

**systemd unit** (`deploy/abshare.service`)
- `DynamicUser=yes`, `StateDirectory=abshare` (0700), `UMask=0077`.
- `ProtectSystem=strict`, `ProtectHome=yes`, `PrivateTmp=yes`, `PrivateDevices=yes`, `ProtectKernelTunables=yes`, `ProtectKernelModules=yes`, `ProtectKernelLogs=yes`, `ProtectControlGroups=yes`, `ProtectClock=yes`, `ProtectHostname=yes`, `ProtectProc=invisible`, `ProcSubset=pid`.
- `ReadOnlyPaths=<library roots>`, `NoNewPrivileges=yes`, `CapabilityBoundingSet=` (none), `AmbientCapabilities=` (none).
- `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`.
- `IPAddressDeny=any` with `IPAddressAllow=` the proxy's address and localhost, so there is no outbound network at the kernel level.
- `SystemCallFilter=@system-service`, `SystemCallArchitectures=native`, `MemoryDenyWriteExecute=yes`, `LockPersonality=yes`, `RestrictNamespaces=yes`, `RestrictRealtime=yes`, `RestrictSUIDSGID=yes`, `RemoveIPC=yes`.
- `MemoryMax=` and `CPUQuota=` (suggested values in DEPLOY).
- The target is a `systemd-analyze security` exposure score of **≤ 1.5**, checked in CI against the unit file.
- ffmpeg children inherit all of this, inside their own jail.

**Docker**
- Multi-stage build: static musl `abshare` and `abshare-jail`, the web build, and a minimal static ffmpeg/ffprobe ≥ 7.1 built from a pinned source tarball.
- Runtime image is `distroless/static:nonroot`, run as a non-root user, with a read-only root filesystem, `--cap-drop ALL`, `no-new-privileges`, the default seccomp profile, `/data` as a volume, the library mounted `:ro`, and no published port except to the proxy's network.
- An image scan (Trivy or Grype) runs in CI and fails on high or critical findings.

**Secrets**
- `secret.key` (32 bytes, 0600) is created at first start from the OS CSPRNG.
- It derives (HKDF with distinct labels) the HMAC key for IDs, the TOTP-seed encryption key, the ticket MAC key and the audit-chain key.
- Losing it invalidates TOTP enrolments and book IDs, so DEPLOY says to back it up with the DB, separately from DB-only backups.

**Logging**
- `tracing` with no tokens, cookies, invite codes, passwords or TOTP codes in any line. Redacted `Debug` impls are tested.
- Paths appear only in admin-level scan diagnostics.

**Privacy**
- No telemetry and no outbound connections from the server.
- PRIVACY.md lists what is stored, where and for how long.

### 9.4 Security process

- **Threat model first.** Every new surface, decision or dependency updates `docs/security/threat-model.md` and the ASVS map (D20) in the same PR.
- **A security gate on every milestone** (§11), not only at the end. Each gate has:
  - a self-review against the ASVS map;
  - a mutation check (remove each protection and confirm a test fails);
  - a fuzz campaign;
  - findings written up in `docs/security/`.
- **External-style review before exposure.** As with lyrion-anywhere's full review and pentest: a full code review and a black-box pentest of the deployed stack (proxy included) before D22's public exposure. Every critical, high and medium finding is fixed with a failing-first regression test.
- **Disclosure.** SECURITY.md covers private vulnerability reporting (GitHub advisories), supported versions and response targets.
- **Dependencies.** New crates and npm packages need a justification in the PR (why, alternatives, maintenance, `unsafe` use). The tree is kept small, and `cargo tree -d` duplicates are reviewed at each release.
- **Honesty.** Docs claim only what is built and tested (lyrion-anywhere review M18).

---

## 10. Native apps (post-v1)

- The API is designed so that a native client can use it unchanged:
  - the refresh token comes in the body for non-browser clients (`X-ABShare-Client: native`);
  - media and downloads accept `Authorization: Bearer`;
  - passkeys work through the platform APIs.
- Not planned until the PWA has proved itself.

---

## 11. Milestones

| Phase | Contents | Gate |
|---|---|---|
| **0 — Spikes** | (a) Offline playback options A and B (§8.3) on iOS and Android with a 2.5 GB book. (b) The `<audio>` + Range + Media Session lock-screen behaviour over 2 h on iOS PWA and Android. (c) A minimal static ffmpeg ≥ 7.1 build; decodes xHE-AAC; size and licence check. (d) `abshare-jail`: Landlock + seccomp around ffmpeg; prove the escape tests fail closed; check kernel support on the target host. (e) The `webauthn-rs` passkey flow in Safari, Chrome and Firefox. (f) Data saver (D23): a Rust implementation of the segment maths and server; seeking anywhere in an 85 h virtual MP3 (about 1.2 GB `Content-Length`) on iOS Safari (installed PWA, screen locked), Android Chrome, desktop Firefox and Chrome; a blind listening test at 32/40/48 kb/s on real narration; check segment cost on the target host. (g) Landlock and the jail's seccomp filter working inside Docker inside an unprivileged LXC (D24). | A written report (`docs/phase0-report.md`); chosen options recorded as decisions. |
| **1 — Core and scanner** | Workspace and CI; threat model and ASVS map; `core` metadata, assembly, chapters and progress-merge with fixtures for every library shape in §4.3; scanner, probe through the jail, catalogue, covers; synthetic demo library; fuzz targets. | The scanner handles the demo library and a private real-library dry run, with problems listed and no panics. Fuzz targets run clean for 1 h each. Jail escape tests pass. |
| **2 — Auth** | Invites, enrolment, passwords, TOTP, passkeys, recovery codes, sessions, devices, lockout, step-up, audit chain; CLI bootstrap. | The negative-test suite for every auth route; mutation check; ASVS authentication and session self-review written up. |
| **3 — Streaming and the app** | Media route, playable-copy jobs, the PWA (Home, Library, Book, Player, Settings), Media Session, progress sync. | A phone keeps playing for 2 h with the screen locked. Resume across two devices is within 15 s. Range and access-control negative tests. CSP and Trusted Types are clean in all three engines. |
| **4 — Offline** | Downloads (original and data saver), chunked IndexedDB, offline playback, outbox sync, save file. | Airplane-mode listening for 1 h, then reconnect: progress merges correctly. A 2.5 GB download resumes after the app is killed. Cross-account isolation tests on one browser. |
| **5 — Admin and deploy** | Admin UI, metadata overrides, jobs, cache and security-status views; Docker image; systemd unit; nginx and Caddy examples; `doctor`; DEPLOY, SECURITY and PRIVACY docs. | The CI deploy test (image behind nginx: TLS redirect, single CSP, cookie flags, spoofed XFF refused, `Host` check, `/v1` unreachable without auth). `systemd-analyze security` ≤ 1.5. The image scan is clean. |
| **6 — Review and release** | Full code review and black-box pentest (§9.4); fixes with failing-first tests; v1.0.0. | No open critical, high or medium findings. **Only then is it exposed to the internet (D22).** |

---

## 12. Open questions

- [ ] **Data-saver bitrate:** 32 kb/s (default), 40 or 48? This is settled by the Phase 0(f) listening test on real narration, including full-cast productions with music.
- [ ] **Multi-part MP3 books:** transcode them to AAC in a playable copy (simple, as specified), or serve a virtual byte-concatenation of the MP3 frames (no transcode and no extra disk, but VBR seeking is risky)?
- [ ] **Sharing between users:** recommendations or shared progress? Out of v1 unless wanted.
- [ ] **Network-level gate in front:** should DEPLOY recommend an extra layer for the owner's install (e.g. a geo-block or allow-list at the proxy, or mTLS for admin routes)? It wouldn't replace anything above.

---

## 13. Testing strategy (standing rule)

Every change ships with tests for security, stability and reliability. Security is the top priority.

| Layer | Security | Stability | Reliability |
|---|---|---|---|
| `core` | Validators refuse traversal-looking or invisible-character input; HTML-to-text never emits markup | Property tests: book assembly, chapter clamping, progress merge (idempotent for duplicates, monotonic in event time) | Fixtures for every library shape and tag quirk (sentinel `part`, comma-joined people, HTML descriptions, conflict-copy names, Unicode, colons in names) |
| Parsers (ffprobe JSON, cue, tags, HTML-to-text, images) | `cargo-fuzz` targets (D21) plus robustness loops: never panic, bounded memory and time | Truncated or oversize inputs | — |
| `server` routes | Negative tests per route: no, expired, forged and revoked tokens; wrong user; other library → 404; CSRF/Origin; wrong `Host`; oversize bodies; rate limits; spoofed XFF; ID fuzzing; a routing-table test that every route declares auth | Range edge cases (suffix, open-ended, 416, multi-range) | Integration with a fake library of synthetic audio: scan → stream → progress → offline ticket |
| Auth | Lockout can't be bypassed in parallel; user lockout can't be triggered by strangers; TOTP replay refused; passkey origin and RP mismatch refused; refresh reuse revokes; step-up enforced; absolute lifetime enforced | Clock-skew windows | Session expiry and refresh under concurrent tabs |
| Jail and ffmpeg | Escape tests: read outside inputs, write outside the job dir, open a socket, start another program; all refused. Forced demuxer: an HLS/concat-disguised file is refused. Size cap and timeout kill. | Corrupt and truncated media | Job queue survives restart; no half files after a kill |
| Data saver | The virtual length and every frame header are checked; a malformed segment is never served; Range requests beyond the end get 416; per-user segment caps | Property test: any byte range maps to the right segments and frames | Joins: decoded output across every boundary of a synthetic book matches a continuous encode in timing (0-sample lag) with no discontinuity above the signal's own; quality switch keeps position within 50 ms |
| Web | No-HTML-injection scan; CSP build check; Trusted Types; contrast tests; 44 px targets | Player state machine (source change, stall, abort, 401) | Offline outbox replay; download resume |
| Deploy | Docker + nginx test script; `systemd-analyze security`; image scan | — | — |

Further rules:
- Security fixes get a regression test that fails before the fix.
- Mutation checks: each security control has a test that fails when the control is removed.
- ffmpeg and jail tests can't skip silently in CI (`ABSHARE_REQUIRE_FFMPEG=1`, `ABSHARE_REQUIRE_LANDLOCK=1`).
- Opt-in live tests read gitignored `dev/local.env` and never write to the library.
- No work is called done while any test is failing or skipped.

---

## 14. Project conventions

- **No personal or homelab information in the repo.** No IP addresses, hostnames, domains, real library contents (titles, paths, counts), credentials or emails, in code, tests, fixtures, docs, commits or PR descriptions.
  - Use placeholders: `books.example.com`, `10.0.0.10`, `/srv/audiobooks`, invented titles.
  - Real values go only in gitignored `dev/local.*` and `.env.local`.
  - Scan the staged diff before every commit.
- British English in docs and UI strings.
- Clean-room. Audiobookshelf, Booksonic and others are references for behaviour only; never copy their code.
- Decisions are recorded in §1 with a date and a reason. Changing one updates the row ("Changed from X on <date>").
- CI:
  - `cargo fmt --check` and `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cargo test`, `cargo deny check` and short fuzz runs;
  - the web check and tests;
  - the docs link check, the deploy test and the image scan;
  - actions pinned by SHA, `permissions: contents: read`.
- CHANGELOG.md has dated entries, newest first, each with a **Tests:** line.

---

## 15. References

- lyrion-anywhere: the security model, ffmpeg hardening, PWA and service-worker policy, and test conventions this project inherits.
- OWASP ASVS 5.0; OWASP Password Storage and Session Management Cheat Sheets; RFC 6238 (TOTP); W3C WebAuthn Level 3.
- Linux Landlock (kernel docs, ABI versions); seccomp-bpf; the systemd.exec manual's sandboxing options.
- Media Session API; WebKit notes on background audio and storage for home-screen web apps.
- FFmpeg 7.1 release notes (native USAC/xHE-AAC decoder).
