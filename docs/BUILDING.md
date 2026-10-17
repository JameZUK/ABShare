# Building and testing

How to build ABShare from source, run every check, try it locally with invented books, and contribute. To install it for real, see [`DEPLOY.md`](DEPLOY.md).

## Requirements

- **Linux** with Landlock enabled (kernel 6.12 or later, `landlock` in the active LSM list). Without it everything builds and most tests pass, but the jail and the tests that run ffmpeg through it are skipped, and media jobs fail at run time.
- **Rust 1.99** or later (edition 2024), with `rustfmt` and `clippy`.
- **Node.js 24** and npm, for the web app.
- **ffmpeg and ffprobe 7.1** or later, for the scanner and media tests.
- **SQLite** is bundled (via `rusqlite`); nothing to install.
- For fuzzing: a nightly toolchain and `cargo install cargo-fuzz`.
- For the image: Docker with BuildKit.

## Building

```sh
# The server and the jail
cargo build --release -p abshare-server -p abshare-jail
# → target/release/abshare and target/release/abshare-jail

# The web app
cd web/app
npm ci --ignore-scripts
npm run build          # → web/app/build, served by abshare when web_dir points at it
```

The release profile uses LTO, one codegen unit and `panic = "abort"`.

### The Docker image

```sh
docker build -f deploy/Dockerfile -t abshare:latest .
```

The Dockerfile has three stages: a static musl build of `abshare` and `abshare-jail` (with OpenSSL vendored for the passkey library), the web build, and a Debian runtime with ffmpeg. It runs as uid 65534.

## Checks

`tools/check.sh` runs every local gate and stops at the first failure:

```sh
tools/check.sh > check.log 2>&1; echo "exit $?"
```

1. `cargo fmt --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. In `web/app`: `npm run check` (svelte-check with warnings as errors), `npm test` (Vitest and the Node tests, including the HTML-injection source scan), and `npm run build` (which fails on any inline script, `on*=` attribute or `javascript:` URL).

Check its exit status rather than reading the end of the log: a piped `| tail` hides failures.

Set `ABSHARE_REQUIRE_LANDLOCK=1` to make the jail and ffmpeg tests fail instead of skipping when the sandbox isn't available.

### Continuous integration

`.github/workflows/ci.yml` runs on every push and pull request, with actions pinned by SHA and read-only permissions:

| Job | What |
|---|---|
| `test` | Formatting, clippy, and the Rust tests in debug and release builds, with ffmpeg installed. |
| `web` | `npm ci`, the type check, the tests and the build. |
| `supply-chain` | `cargo deny check`: licences, advisories, banned crates and sources. |
| `fuzz` | Every fuzz target for 60 seconds. |
| `image` | Builds the image, runs it behind nginx with TLS and checks it from outside (`deploy/ci/run.sh`); scans the image with Trivy for high and critical vulnerabilities; checks that both systemd units score 1.5 or less with `systemd-analyze security`; checks that the image runs as non-root. |

### Fuzzing

```sh
cargo +nightly fuzz list
cargo +nightly fuzz run metadata -- -max_total_time=600 -dict=fuzz/dicts/metadata.dict
```

There are 11 targets, one for each parser of untrusted bytes. Results and how to run a long campaign are in [`fuzzing.md`](fuzzing.md). A crash becomes a regression test.

### The mutation check

`tools/mutation-check.py` removes each security control in turn and confirms that at least one test fails. Run it after changing a control.

## A local demo

Two scripts build synthetic libraries of invented books (no real titles, people or audio):

- `tools/demo/make-library.sh OUT` covers every library shape and quirk the scanner must handle. It is for tests.
- `tools/demo/make-showcase.sh OUT` builds a good-looking library across several genres, with two series, a podcast-style show and titled covers. The screenshots in these docs come from it.

```sh
tools/demo/make-showcase.sh /tmp/abshare-demo/lib
chmod -R a-w /tmp/abshare-demo/lib          # the library must be read-only
mkdir -m 700 /tmp/abshare-demo/data

# Run from the repository root: the paths below must be absolute.
cat > /tmp/abshare-demo/abshare.toml <<EOF
listen = "127.0.0.1:8743"
data_dir = "/tmp/abshare-demo/data"
public_origin = "http://localhost:8743"
web_dir = "$PWD/web/app/build"

[ffmpeg]
ffmpeg = "/usr/bin/ffmpeg"
ffprobe = "/usr/bin/ffprobe"
jail = "$PWD/target/release/abshare-jail"
libs = ["/usr/lib"]

[[library]]
name = "Audiobooks"
path = "/tmp/abshare-demo/lib"
EOF
chmod 600 /tmp/abshare-demo/abshare.toml

C=/tmp/abshare-demo/abshare.toml
target/release/abshare scan --config $C --problems
target/release/abshare admin invite --config $C --admin --name "Ada Lovelace"
target/release/abshare serve --config $C
```

Open the printed invite link. `http://localhost` is the only plain-HTTP origin ABShare accepts, and it counts as secure for passkeys in most browsers. `abshare doctor --config $C` checks the set-up.

### Developing the web app

With `abshare serve` running on port 8743, the Vite dev server proxies `/v1` and `/healthz` to it:

```sh
cd web/app
npm run dev            # or ABSHARE_SERVER=http://127.0.0.1:8790 npm run dev
```

Add the dev server's origin as `public_origin` while you do this, since the server checks `Host` and `Origin`.

## Contributing

The working rules are in [`CLAUDE.md`](../CLAUDE.md) and [`SPEC.md`](../SPEC.md) §14. In short:

- **Security first.** Every route declares its auth, every body has a strict schema, and every externally reachable change comes with negative tests that prove attacks are refused. Security fixes come with a regression test that fails before the fix.
- **Tests at every point.** No component, endpoint or parser lands without tests, and every parser of untrusted bytes has a fuzz target. Work isn't done while any test fails or is skipped.
- **`core` does no I/O.** Pure logic goes in `core`; I/O goes in `server`.
- **No personal information.** This repository is public. Use placeholders (`books.example.com`, `10.0.0.10`, `/srv/audiobooks`, invented titles). `tools/check-private.py` runs as a pre-commit hook against a gitignored list of private strings; never bypass it.
- **Clean room.** Other audiobook servers are references for behaviour only; never copy their code.
- **New dependencies** need a reason in the pull request: why, the alternatives, maintenance and `unsafe` use.
- **British English** in docs and the interface. Docs claim only what is built and tested.
- **Decisions** are recorded in `SPEC.md` §1 with a date and a reason.
