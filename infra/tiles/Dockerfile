# syntax=docker/dockerfile:1
#
# The basemap service: `pmtiles serve` plus a first-start step that fetches
# the map data (see entrypoint.sh). Build context: this directory.

ARG PMTILES_VERSION=1.31.2

FROM alpine:3.24 AS fetch
ARG PMTILES_VERSION
ARG TARGETARCH
RUN apk add --no-cache curl tar \
 && case "$TARGETARCH" in arm64) arch=arm64 ;; *) arch=x86_64 ;; esac \
 && curl -fsSL "https://github.com/protomaps/go-pmtiles/releases/download/v${PMTILES_VERSION}/go-pmtiles_${PMTILES_VERSION}_Linux_${arch}.tar.gz" \
    | tar -xz -C /usr/local/bin pmtiles

FROM alpine:3.24
RUN apk add --no-cache ca-certificates curl tar \
 && adduser -D -u 10001 tiles \
 && mkdir -p /data /assets && chown tiles:tiles /data /assets
COPY --from=fetch /usr/local/bin/pmtiles /usr/local/bin/pmtiles
COPY --chmod=0755 entrypoint.sh /usr/local/bin/entrypoint.sh
USER tiles
VOLUME ["/data", "/assets"]
EXPOSE 8080
# Healthy once the world archive is there and a tile can be read from it.
HEALTHCHECK --interval=15s --timeout=5s --start-period=30m \
  CMD curl -fsS -o /dev/null http://127.0.0.1:8080/world/0/0/0.mvt || exit 1
ENTRYPOINT ["entrypoint.sh"]
