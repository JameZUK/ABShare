# ABShare server image: the server, the jail and the web app. Build from the repository root:
#   docker build -f deploy/Dockerfile -t abshare:dev .
#
# Runtime contract: run as a non-root user, read-only root filesystem, all capabilities
# dropped, no-new-privileges, the library mounted read-only, /data writable (0700).
#
# Every base image is pinned by digest (SPEC §9.3, §10). To update one, resolve the tag's
# current multi-platform index digest and replace the part after "@":
#   docker buildx imagetools inspect rust:1.99-alpine    (the "Digest:" line)
# then rebuild and run tools/check.sh and deploy/ci/run.sh.

FROM rust:1.99-alpine@sha256:557a94c82b485a14b569120e00867267108c734efe33f10cc31ca0cd5dd1eca9 AS build
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
FROM node:24-alpine@sha256:44fce696f38ba8c987c4db89954051000bbd05ffd8d8659ba15ed4a8602e937f AS web
WORKDIR /app
COPY web/app/package.json web/app/package-lock.json ./
RUN npm ci --ignore-scripts
COPY web/app ./
RUN npm run build

# A minimal, statically linked ffmpeg and ffprobe (SPEC D10, §9.3), from pinned source
# tarballs checked by SHA-256. The FFmpeg checksum was taken from a tarball whose release
# signature (key FCF9 86EA 15E6 E293 A564 4F10 B432 2F04 D676 58D8) verified. To update:
# download the new tarball and its .asc from ffmpeg.org/releases, verify the signature,
# then change FFMPEG_VERSION and FFMPEG_SHA256 together.
#
# Only what ABShare runs is built (server/src/{probe,covers,media,tools}.rs):
#   inputs     the forced demuxers mov, mp3, ogg, flac and wav (tools::demuxer_for), and
#              lavfi for the generated silence (media::silent_frame);
#   decoders   AAC (LC/HE), ALAC, MPEG audio layers 1-3, Vorbis, Opus, FLAC and plain PCM;
#              cover pictures are copied, never decoded;
#   outputs    AAC in MP4 (playable copies), LAME MP3 (data saver), image2 (covers) and
#              null with PCM (the trial decode);
#   filters    concat, aresample, asettb, atrim, asetpts, anullsrc, plus the ones ffmpeg
#              inserts itself;
#   protocols  file only: no network protocol is compiled in, and the build fails if one is.
# No GPL parts: FFmpeg stays LGPL 2.1+, as does LAME.
FROM alpine:3.24@sha256:294b683cb724975bec92580e1e685676bd4b50bda910ddb8c51d4cabeaec77e6 AS ffmpeg
ARG FFMPEG_VERSION=7.1.5
ARG FFMPEG_SHA256=de668509caf9e35e3cd162473441fdb29538c6d96ed080292b3cf9e6fc5d558f
ARG LAME_VERSION=3.100
ARG LAME_SHA256=ddfe36cab873794038ae2c1210557ad34857a4b6bdc515785d1da9e175b1da1e
RUN apk add --no-cache build-base nasm curl xz file
WORKDIR /build
RUN curl -fsSL --proto '=https' --tlsv1.2 -o ffmpeg.tar.xz "https://ffmpeg.org/releases/ffmpeg-${FFMPEG_VERSION}.tar.xz" \
 && curl -fsSL --proto '=https' --tlsv1.2 -o lame.tar.gz "https://downloads.sourceforge.net/project/lame/lame/${LAME_VERSION}/lame-${LAME_VERSION}.tar.gz" \
 && echo "${FFMPEG_SHA256}  ffmpeg.tar.xz" | sha256sum -c - \
 && echo "${LAME_SHA256}  lame.tar.gz" | sha256sum -c - \
 && tar -xJf ffmpeg.tar.xz && tar -xzf lame.tar.gz
# LAME 3.100 predates C23, the default of current GCC.
RUN cd "lame-${LAME_VERSION}" \
 && CFLAGS="-O2 -std=gnu17 -fstack-protector-strong -D_FORTIFY_SOURCE=2 -fPIE" ./configure \
      --prefix=/opt/lame --enable-static --disable-shared --disable-frontend --disable-decoder \
      --disable-gtktest --disable-dependency-tracking \
 && make -j"$(nproc)" && make install
# musl's default thread stack (128 KiB) is too small for some codec threads: ask for 2 MiB.
RUN cd "ffmpeg-${FFMPEG_VERSION}" \
 && ./configure \
      --prefix=/opt/ffmpeg --toolchain=hardened \
      --extra-cflags="-I/opt/lame/include" \
      --extra-ldflags="-L/opt/lame/lib -static -Wl,-z,stack-size=2097152" \
      --pkg-config-flags=--static \
      --enable-static --disable-shared --disable-debug --disable-doc \
      --disable-everything --disable-autodetect --disable-network \
      --disable-ffplay --disable-swscale --disable-postproc \
      --enable-libmp3lame \
      --enable-protocol=file \
      --enable-demuxer=mov,mp3,ogg,flac,wav \
      --enable-indev=lavfi \
      --enable-decoder=aac,alac,mp1,mp1float,mp2,mp2float,mp3,mp3float,vorbis,opus,flac \
      --enable-decoder=pcm_s16le,pcm_s16be,pcm_s24le,pcm_s24be,pcm_s32le,pcm_s32be,pcm_u8 \
      --enable-decoder=pcm_f32le,pcm_f32be,pcm_f64le,pcm_f64be,pcm_alaw,pcm_mulaw \
      --enable-parser=aac,mpegaudio,flac,vorbis,opus \
      --enable-encoder=aac,libmp3lame,pcm_s16le \
      --enable-muxer=mp4,mp3,image2,null \
      --enable-filter=aresample,aformat,anull,asettb,atrim,asetpts,concat,anullsrc \
      --enable-bsf=null \
 && make -j"$(nproc)" && make install \
 && strip /opt/ffmpeg/bin/ffmpeg /opt/ffmpeg/bin/ffprobe \
 && for b in ffmpeg ffprobe; do \
      file "/opt/ffmpeg/bin/$b" | grep -Eq "static(-pie)? linked|statically linked" || { file "/opt/ffmpeg/bin/$b"; exit 1; }; \
    done \
 && protocols="$(/opt/ffmpeg/bin/ffmpeg -hide_banner -protocols | grep -v -e '^Supported file protocols:' -e '^Input:' -e '^Output:' | tr -d ' ' | sort -u | tr '\n' ' ')" \
 && [ "$protocols" = "file " ] || { echo "unexpected protocols: $protocols"; exit 1; }

# The runtime: distroless static (SPEC §10). Every binary is ours and statically linked, so
# the image holds them, the web app, and the few files distroless brings (/etc/passwd, CA
# certificates, time zones): no shell, no package manager, no other programs to misuse.
# Use the commands in docs/DEPLOY.md, which run the binaries directly. It runs as nobody
# (65534), as before, so existing data directories keep working.
FROM gcr.io/distroless/static-debian13:nonroot@sha256:e2e927ec666bae08560abb3c55d0659eceabb657f56b6782ab500a9fc7f555e3
COPY --from=build /src/target/release/abshare /src/target/release/abshare-jail /usr/local/bin/
COPY --from=ffmpeg /opt/ffmpeg/bin/ffmpeg /opt/ffmpeg/bin/ffprobe /usr/bin/
COPY --from=web /app/build /usr/share/abshare/web
USER 65534:65534
VOLUME ["/data"]
ENTRYPOINT ["/usr/local/bin/abshare"]
CMD ["check-config", "--config", "/etc/abshare/abshare.toml"]
