# Deploying ABShare

ABShare runs as two processes from one image: the web server (`abshare serve`) and the
library scanner (`abshare scan --every 3600`). Put a TLS reverse proxy in front.

## Requirements

- **Linux 6.12 or later with Landlock enabled** (Landlock ABI 6). ABShare refuses to run
  ffmpeg without its sandbox. Check with `cat /sys/kernel/security/lsm`: the list must include
  `landlock`. If it doesn't, add `lsm=landlock,` followed by the existing list to the kernel
  command line and reboot.
- Docker with Compose, or systemd.
- About 2 GB of memory; disk for the transcode cache (10 GiB by default, `[cache] max_gib`).
- Your audiobook folder, **mounted read-only**. ABShare refuses a library it can write to.

## Docker Compose (recommended)

```sh
mkdir -p /srv/abshare/data
chown 65534:65534 /srv/abshare/data && chmod 0700 /srv/abshare/data
cd /srv/abshare
cp /path/to/abshare/deploy/compose.example.yaml compose.yaml
cp /path/to/abshare/deploy/abshare.example.toml abshare.toml
```

Edit `abshare.toml`:

- `public_origin`: the address people use, such as `https://books.example.com`;
- `trusted_proxies`: your reverse proxy's address, as ABShare sees it;
- the `[[library]]` path, matching the volume in `compose.yaml` (`/library`).

Edit `compose.yaml` so the library volume points at your audiobook folder, with `:ro`.

Then:

```sh
docker build -f deploy/Dockerfile -t abshare:latest /path/to/abshare
docker compose run --rm --entrypoint /usr/local/bin/abshare scan doctor --config /etc/abshare/abshare.toml
docker compose up -d
```

`doctor` checks the configuration, permissions, that the library is read-only, Landlock, the
jailed ffmpeg, the database and the audit log. Fix anything marked FAIL before going on.

The image is distroless: it has no shell, so `docker exec … sh` doesn't work. Run any command
as above, by calling `/usr/local/bin/abshare` directly.

The first scan starts on its own; a large library takes a few minutes the first time and
seconds afterwards. To scan now: Settings → Admin → Library → Scan now.

### The first admin

```sh
docker compose run --rm --entrypoint /usr/local/bin/abshare scan admin invite --config /etc/abshare/abshare.toml --admin
```

Open the printed link (it works once, for 72 hours) and create a passkey. Invite everyone
else from Settings → Admin → Invites.

## Reverse proxy

Examples: [`deploy/nginx.example.conf`](../deploy/nginx.example.conf) and
[`deploy/Caddyfile.example`](../deploy/Caddyfile.example). What matters:

- **TLS** in front; plain HTTP only to redirect.
- The proxy **overwrites** `X-Forwarded-For` (with the client address) and sets
  `X-Forwarded-Proto: https`. ABShare believes these headers only from addresses in
  `trusted_proxies`, and uses them for lockouts and rate limits.
- `Host` passed through unchanged. ABShare answers only for the host in `public_origin`.
- **No `Content-Security-Policy` at the proxy.** ABShare sends its own; two policies combine
  and break the app.
- No response buffering for audio, and long read timeouts.

## systemd (without Docker)

[`deploy/abshare.service`](../deploy/abshare.service) and
[`deploy/abshare-scan.service`](../deploy/abshare-scan.service) are hardened units
(`systemd-analyze security` scores 1.4 and 0.6). Install the binaries in `/usr/local/bin`, the
built web app (`web/app/build`) in `/usr/share/abshare/web`, and the configuration in
`/etc/abshare/abshare.toml` with `data_dir = "/var/lib/abshare"`.

## Upgrading

Build or pull the new image, then `docker compose up -d`. The database migrates itself on
start. Open apps show "An update is ready" and reload when the listener chooses.

## Backups

Back up the data directory: `abshare.db` (accounts, progress, bookmarks) and `secret.key`
(it seals authenticator secrets and signs the audit log). Keep `secret.key` separate from
database backups if you can. The transcode cache (`saver/`, `copies/`) and `covers/` can
be rebuilt and needn't be backed up.

## Checklist

- [ ] `abshare doctor` shows no FAIL.
- [ ] The library is mounted read-only.
- [ ] Only the reverse proxy can reach port 8743.
- [ ] `trusted_proxies` lists the proxy, and nothing else.
- [ ] There are at least two admins, so losing one account isn't a lockout.
