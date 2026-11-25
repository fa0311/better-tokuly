# syntax=docker/dockerfile:1
#
# The `tokuly` backend image. One image runs every backend role
# (`api`, `realtime`, `worker`, `migrate`); compose picks the role per service.
# Build context: the repository root.

ARG RUST_VERSION=1.100.0
ARG BUF_VERSION=1.75.0

FROM bufbuild/buf:${BUF_VERSION} AS buf

FROM rust:${RUST_VERSION}-slim-trixie AS build
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential cmake pkg-config clang perl \
 && rm -rf /var/lib/apt/lists/*
# The proto crate shells out to `buf build` at compile time.
COPY --from=buf /usr/local/bin/buf /usr/local/bin/buf
WORKDIR /src
COPY buf.yaml ./
COPY proto ./proto
COPY backend ./backend
WORKDIR /src/backend
# Parallel rustc jobs; CI passes a small number, since compiling the generated
# API crate in release mode next to other crates ran a 16 GB runner out of memory.
ARG CARGO_BUILD_JOBS=default
# Cache mounts keep the crate registry and build artifacts between image builds.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/backend/target,sharing=locked \
    cargo build --release --locked -p tokuly-server \
 && install -m 0755 target/release/tokuly /usr/local/bin/tokuly

# Static ffmpeg/ffprobe (libx264, SVT-AV1, Opus...) for the media pipeline.
FROM mwader/static-ffmpeg:8.0 AS ffmpeg

FROM debian:trixie-slim AS runtime
LABEL org.opencontainers.image.source="https://github.com/fa0311/better-tokuly" \
      org.opencontainers.image.title="Tokuly backend" \
      org.opencontainers.image.description="The Tokuly server: API, realtime gateway, workers and migrations in one binary."
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /var/lib/tokuly --create-home tokuly \
 # Where the media server's recordings are mounted (worker). Owned by our
 # user so that a fresh volume mounted here is too.
 && install -d -o tokuly -g tokuly /recordings
COPY --from=ffmpeg /ffmpeg /ffprobe /usr/local/bin/
COPY --from=build /usr/local/bin/tokuly /usr/local/bin/tokuly
USER tokuly
WORKDIR /var/lib/tokuly
ENV TOKULY_LOG_FORMAT=json
ENTRYPOINT ["tini", "--", "tokuly"]
CMD ["api"]
