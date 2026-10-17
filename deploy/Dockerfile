# ABShare server image: the server, the jail and the web app. Build from the repository root:
#   docker build -f deploy/Dockerfile -t abshare:dev .
#
# Runtime contract: run as a non-root user, read-only root filesystem, all capabilities
# dropped, no-new-privileges, the library mounted read-only, /data writable (0700).
# Base images are tags for now; the release image pins every base by digest (SPEC 9.3).

FROM rust:1.99-alpine AS build
# perl and make build the vendored OpenSSL (webauthn-rs, D25).
RUN apk add --no-cache musl-dev file perl make
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY core core
COPY jail jail
COPY server server
COPY tools/saver-spike tools/saver-spike
RUN cargo build --release --locked -p abshare-server -p abshare-jail \
 && for b in abshare abshare-jail; do \
      file "target/release/$b" | grep -Eq "static(-pie)? linked|statically linked" || { file "target/release/$b"; exit 1; }; \
    done

# The web app: static files, served by the server from web_dir.
FROM node:24-alpine AS web
WORKDIR /app
COPY web/app/package.json web/app/package-lock.json ./
RUN npm ci --ignore-scripts
COPY web/app ./
RUN npm run build

# Debian trixie's ffmpeg 7.1 for now; a minimal ffmpeg build of our own replaces it later.
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg \
 && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/abshare /src/target/release/abshare-jail /usr/local/bin/
COPY --from=web /app/build /usr/share/abshare/web
USER 65534:65534
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/abshare"]
CMD ["check-config", "--config", "/etc/abshare/abshare.toml"]
