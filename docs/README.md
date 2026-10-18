# Documentation

Every page of ABShare's documentation, with what each covers. Start with the project [`README`](../README.md) for an introduction.

## Using and running it

| Document | For | What it covers |
|---|---|---|
| [`DEPLOY.md`](DEPLOY.md) | Administrators | Installing with Docker Compose or systemd, the read-only library, the reverse proxy, the first admin, upgrades and backups |
| [`FEATURES.md`](FEATURES.md) | Everyone | Every feature by area, and what isn't built yet |
| [`PLATFORMS.md`](PLATFORMS.md) | Everyone | Supported browsers and devices, what has been tested, project status and roadmap |
| [`../SECURITY.md`](../SECURITY.md) | Everyone | The security model in brief, review status, and how to report a vulnerability |
| [`PRIVACY.md`](PRIVACY.md) | Everyone | What ABShare stores, where, and for how long |

## How it works

| Document | What it covers |
|---|---|
| [`ARCHITECTURE.md`](ARCHITECTURE.md) | Components, repository layout, the trust boundary, scanning and book assembly, request flows with diagrams, the ffmpeg jail, media jobs and the cache, the web app, storage, configuration and testing |
| [`API.md`](API.md) | The HTTP API (`/v1`) as built: conventions, errors, authentication, your account, the library, progress, media, admin |
| [`PERFORMANCE.md`](PERFORMANCE.md) | Memory, streaming, data-saver and offline measurements, and how to take them |
| [`../SPEC.md`](../SPEC.md) | The specification: every design decision (D1–D27), goals, the threat model, milestones and the testing strategy |
| [`security/`](security) | The threat model, the OWASP ASVS map, and the Phase 2 and Phase 6 reviews |
| [`fuzzing.md`](fuzzing.md) | Fuzz targets and campaign results |
| [`phase0-report.md`](phase0-report.md) | The Phase 0 spikes: offline playback, background audio and data saver on real devices |

## Building and contributing

| Document | What it covers |
|---|---|
| [`BUILDING.md`](BUILDING.md) | Building from source, every check, CI, fuzzing, a local demo with invented books, and the rules for contributing |
| [`../CHANGELOG.md`](../CHANGELOG.md) | What changed and when, newest first |
