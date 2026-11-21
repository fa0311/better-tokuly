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
# Cache mounts keep the crate registry and build artifacts between image builds.
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/backend/target,sharing=locked \
    cargo build --release --locked -p tokuly-server \
 && install -m 0755 target/release/tokuly /usr/local/bin/tokuly

# Static ffmpeg/ffprobe (libx264, SVT-AV1, Opus...) for the media pipeline.
FROM mwader/static-ffmpeg:8.0 AS ffmpeg

FROM debian:trixie-slim AS runtime
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates tini \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /var/lib/tokuly --create-home tokuly
COPY --from=ffmpeg /ffmpeg /ffprobe /usr/local/bin/
COPY --from=build /usr/local/bin/tokuly /usr/local/bin/tokuly
USER tokuly
WORKDIR /var/lib/tokuly
ENV TOKULY_LOG_FORMAT=json
ENTRYPOINT ["tini", "--", "tokuly"]
CMD ["api"]
