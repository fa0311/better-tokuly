# syntax=docker/dockerfile:1
#
# The `tokuly` backend image. One image runs every backend role
# (`api`, `realtime`, `worker`, `migrate`); compose picks the role per service.
# Build context: the repository root.

ARG RUST_VERSION=1.100.0
ARG BUF_VERSION=1.75.0
ARG CARGO_CHEF_VERSION=0.1.78

FROM bufbuild/buf:${BUF_VERSION} AS buf

FROM rust:${RUST_VERSION}-slim-trixie AS toolchain
ARG CARGO_CHEF_VERSION
RUN apt-get update \
 && apt-get install -y --no-install-recommends build-essential cmake pkg-config clang perl \
 && rm -rf /var/lib/apt/lists/*
RUN cargo install cargo-chef --version "${CARGO_CHEF_VERSION}" --locked
# The proto crate shells out to `buf build` at compile time.
COPY --from=buf /usr/local/bin/buf /usr/local/bin/buf
WORKDIR /src/backend

# The build is split into layers that change at different rates, so that a
# layer cache (CI keeps one in the registry) skips what did not change:
#
#   1. third-party dependencies   change with Cargo.lock
#   2. the generated API crate    changes with proto/ (and is the slowest crate)
#   3. everything else            changes with every commit
#
# cargo-chef writes the recipe for layer 1: the manifests with every crate of
# ours reduced to an empty stub.
FROM toolchain AS planner
COPY backend ./
RUN cargo chef prepare --recipe-path /recipe.json \
 # The real manifests too: the recipe gives our crates a placeholder version,
 # which layers 2 and 3 must not inherit. Copied on, they keep layer 2 cached
 # as long as they are unchanged.
 && find . -path ./target -prune -o \( -name Cargo.toml -o -name Cargo.lock \) -print \
    | tar -cf - -T - | (mkdir /manifests && tar -xf - -C /manifests)

FROM toolchain AS build
# `release` for the published images, `ci` for the ones CI tests against
# (see backend/Cargo.toml).
ARG PROFILE=release
# Parallel rustc jobs; CI passes a small number, since compiling the generated
# API crate in release mode next to other crates ran a 16 GB runner out of memory.
ARG CARGO_BUILD_JOBS=default
COPY --from=planner /recipe.json /recipe.json
RUN cargo chef cook --profile "$PROFILE" --locked -p tokuly-server --recipe-path /recipe.json

COPY --from=planner /manifests/ ./
COPY buf.yaml /src/
COPY proto /src/proto
COPY backend/crates/proto ./crates/proto
# Newer than the stub that cargo-chef compiled in its place.
RUN find crates/proto /src/proto /src/buf.yaml -exec touch {} + \
 && cargo build --profile "$PROFILE" --locked -p tokuly-server

COPY backend ./
# Cargo decides by modification time what to rebuild. The API crate's inputs
# are the ones layer 2 was built from (or it would not have been reused), so
# they are dated back before that build; every other crate of ours is still a
# stub in the target directory and is dated forward, so it is rebuilt.
RUN find crates/proto /src/proto /src/buf.yaml -exec touch -d @0 {} + \
 && find . -path ./target -prune -o -path ./crates/proto -prune -o -type f -exec touch {} + \
 && cargo build --profile "$PROFILE" --locked -p tokuly-server \
 && install -m 0755 "target/$PROFILE/tokuly" /usr/local/bin/tokuly

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
