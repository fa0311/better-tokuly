# syntax=docker/dockerfile:1
#
# PostgreSQL with PostGIS, for amd64 and arm64: the official PostgreSQL image
# plus the PostGIS packages from the PostgreSQL apt repository it already
# uses. (The upstream postgis image is published for amd64 only, which leaves
# Apple Silicon and ARM servers running the database under emulation.)
# Build context: this directory.

ARG POSTGRES_VERSION=18.7

FROM postgres:${POSTGRES_VERSION}-trixie
LABEL org.opencontainers.image.source="https://github.com/fa0311/better-tokuly" \
      org.opencontainers.image.title="Tokuly postgres" \
      org.opencontainers.image.description="PostgreSQL with PostGIS for Tokuly, built for amd64 and arm64."
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      "postgresql-${PG_MAJOR}-postgis-3" "postgresql-${PG_MAJOR}-postgis-3-scripts" \
 && rm -rf /var/lib/apt/lists/*
