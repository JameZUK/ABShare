# ABShare

A private audiobook server for friends and family. Point it at a folder of audiobooks, invite
the people you want to share them with, and they listen in a web app on their phone or
computer: it remembers where everyone is in every book, works with the screen locked, and
downloads books for listening offline.

Security comes first: ABShare is built to face the internet. See [SECURITY.md](SECURITY.md).

## What it does

- **Listen anywhere.** An installable web app (PWA) for iPhone, iPad, Android and desktop:
  lock-screen controls, chapters, speed, sleep timer, bookmarks.
- **Pick up where you left off** on any device: positions sync, including from offline
  listening.
- **Data saver.** A small mono stream (48, 40 or 32 kb/s) for mobile data, with full seeking.
- **Offline.** Download books (original or data saver) and listen with no connection.
- **Browse** by author, series, genre (fiction and non-fiction shelves) and progress; shows
  and podcasts are grouped automatically.
- **Invite-only accounts** with passkeys, or a password with an authenticator app; recovery
  codes; devices you can sign out remotely.
- **Admin screens** for people, invites, library scans, the audit log and security status.
- **Read-only library.** ABShare never writes to your audiobook folder.

## How it's built

- Rust server (axum), SQLite, no outbound network access.
- ffmpeg and ffprobe run only inside `abshare-jail`: Landlock, seccomp and resource limits.
- Pure, fuzzed `core` crate for everything that interprets untrusted bytes (tags, ffprobe
  output, chapters, ranges).
- SvelteKit web app with no runtime dependencies, a strict Content Security Policy and
  Trusted Types.

The design is in [SPEC.md](SPEC.md).

## Running it

See [docs/DEPLOY.md](docs/DEPLOY.md): Docker Compose (recommended) or systemd, behind nginx
or Caddy. Linux 6.12 or later with Landlock enabled is required.

## Documentation

- [docs/DEPLOY.md](docs/DEPLOY.md): installing, the reverse proxy, the first admin, upgrades.
- [SECURITY.md](SECURITY.md): the security model and how to report a vulnerability.
- [docs/PRIVACY.md](docs/PRIVACY.md): what ABShare stores and where.
- [docs/security/](docs/security/): threat model, ASVS map and reviews.
- [docs/fuzzing.md](docs/fuzzing.md): fuzzing results.

## Licence

GPL-2.0-or-later. See [LICENSE](LICENSE).
