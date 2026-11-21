#!/bin/sh
# Prepares the basemap on first start, then serves it.
#
#   /data/world.pmtiles    the whole planet at low zoom (about 50 MB)
#   /data/region.pmtiles   TOKULY_MAP_REGION_BBOX at street level (size depends on the area)
#   /assets                fonts and icons of the map style
#
# Map data: OpenStreetMap via the Protomaps daily builds. Delete a file (or
# the volume) to have it fetched again, for example after changing the region.
set -eu

WORLD_MAXZOOM=6
REGION_BBOX="${TOKULY_MAP_REGION_BBOX:-139.2,35.2,140.3,36.0}"
REGION_MAXZOOM="${TOKULY_MAP_REGION_MAXZOOM:-15}"
ASSETS_URL="https://github.com/protomaps/basemaps-assets/archive/refs/heads/main.tar.gz"

# The newest planet build that exists: builds are published daily and kept for
# about a week, so walk back from today.
latest_build() {
  day=0
  while [ "$day" -lt 10 ]; do
    url="https://build.protomaps.com/$(date -u -d "@$(( $(date -u +%s) - day * 86400 ))" +%Y%m%d).pmtiles"
    if curl -fsSI -o /dev/null "$url"; then
      echo "$url"
      return 0
    fi
    day=$((day + 1))
  done
  return 1
}

fetch_extract() { # <file> <pmtiles extract options...>
  file="$1"; shift
  [ -s "/data/$file" ] && return 0
  source="${TOKULY_MAP_SOURCE:-$(latest_build)}" || { echo "tiles: no planet build reachable; cannot create $file" >&2; return 1; }
  echo "tiles: creating $file from $source ..."
  pmtiles extract "$source" "/data/$file.partial" "$@"
  mv "/data/$file.partial" "/data/$file"
}

fetch_assets() {
  [ -d /assets/fonts ] && [ -d /assets/sprites ] && return 0
  echo "tiles: fetching map fonts and icons ..."
  rm -rf /assets/.partial && mkdir -p /assets/.partial
  curl -fsSL "$ASSETS_URL" | tar -xz -C /assets/.partial --strip-components=1
  rm -rf /assets/fonts /assets/sprites
  mv /assets/.partial/fonts /assets/.partial/sprites /assets/
  rm -rf /assets/.partial
}

# A failed download must not take the map service down with it: whatever is
# already there is served, and the next start tries again.
fetch_extract world.pmtiles --maxzoom="$WORLD_MAXZOOM" || true
fetch_extract region.pmtiles --bbox="$REGION_BBOX" --maxzoom="$REGION_MAXZOOM" || true
fetch_assets || echo "tiles: map fonts and icons could not be fetched" >&2

exec pmtiles serve /data --port 8080 --cors '*'
