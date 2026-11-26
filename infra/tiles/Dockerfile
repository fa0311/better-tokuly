# syntax=docker/dockerfile:1
#
# The basemap service: `pmtiles serve` plus a first-start step that fetches
# the map data (see entrypoint.sh). Build context: this directory.

# Update both architecture checksums below when changing this version.
ARG PMTILES_VERSION=1.31.2

FROM alpine:3.24 AS fetch
ARG PMTILES_VERSION
ARG TARGETARCH
RUN apk add --no-cache curl tar \
 && case "$TARGETARCH" in \
      arm64) arch=arm64; sha256=f8bd47e7ea866863489cad588fbaf2f31f42e5821f7a03f009b3769f05801cb1 ;; \
      amd64) arch=x86_64; sha256=3ed7dbf4ec2e6dfe5e25b6f70d1ffc932729f93c86db353bf514dd71010a312f ;; \
      *) echo "Unsupported architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && curl -fsSL -o /tmp/pmtiles.tar.gz "https://github.com/protomaps/go-pmtiles/releases/download/v${PMTILES_VERSION}/go-pmtiles_${PMTILES_VERSION}_Linux_${arch}.tar.gz" \
 && echo "$sha256  /tmp/pmtiles.tar.gz" | sha256sum -c - \
 && tar -xzf /tmp/pmtiles.tar.gz -C /usr/local/bin pmtiles \
 && rm /tmp/pmtiles.tar.gz

FROM alpine:3.24
LABEL org.opencontainers.image.source="https://github.com/fa0311/better-tokuly" \
      org.opencontainers.image.title="Tokuly tiles" \
      org.opencontainers.image.description="Self-hosted basemap for Tokuly: PMTiles server that fetches its own map data."
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
