# Phase 0(f) data-saver spike image. A test tool: LAN only, no auth. Not the product image.
#
# Build (from the repository root):  docker build -f deploy/spike/Dockerfile -t abshare-saver-spike:phase0 .
#
# Base images are tags here for the spike; the product image pins every base by digest.

FROM rust:1.99-alpine AS build
RUN apk add --no-cache musl-dev file
WORKDIR /src
COPY Cargo.toml Cargo.lock rustfmt.toml ./
COPY core core
COPY jail jail
COPY tools tools
RUN cargo build --release --locked -p abshare-jail -p abshare-saver-spike \
 && for b in abshare-jail abshare-saver-spike; do \
      file "target/release/$b" | grep -Eq "static(-pie)? linked|statically linked" || { file "target/release/$b"; echo "$b is not static"; exit 1; }; \
    done

# Debian trixie's ffmpeg is 7.1 (decodes xHE-AAC). Phase 1 replaces it with a minimal build.
FROM debian:trixie-slim
RUN apt-get update \
 && apt-get install -y --no-install-recommends ffmpeg \
 && rm -rf /var/lib/apt/lists/* \
 && ffmpeg -hide_banner -version | head -1
COPY --from=build /src/target/release/abshare-jail /src/target/release/abshare-saver-spike /usr/local/bin/
USER 65534:65534
EXPOSE 8743
ENTRYPOINT ["/usr/local/bin/abshare-saver-spike", "--jail", "/usr/local/bin/abshare-jail", "--lib", "/usr/lib", "--lib", "/etc/ld.so.cache"]
