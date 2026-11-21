#!/usr/bin/env bash
# Rebuilds the data files bundled with tokuly-geo from their public sources:
#
#   regions.ndjson.gz    country and first-level subdivision boundaries (Natural Earth)
#   gazetteer.tsv.gz     populated places for offline reverse geocoding (GeoNames)
#   admin1.tsv.gz        first-level subdivision names by GeoNames code
#   countries.tsv        country names by ISO 3166-1 alpha-2 code
#
# Usage: data/build.sh [work-dir]      (needs curl, unzip, python3, node/npx)
#
# See README.md in this directory for provenance and licences. The outputs are
# checked in; run this only to refresh them.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
WORK="${1:-$(mktemp -d)}"
NE_TAG="v5.1.2"
NE="https://raw.githubusercontent.com/nvkelso/natural-earth-vector/$NE_TAG/geojson"
GN="https://download.geonames.org/export/dump"
MAPSHAPER="${MAPSHAPER:-npx -y mapshaper@0.6.102}"
# Share of vertices kept by the simplification (keeps the boundaries around 2.5 MB).
KEEP="${KEEP:-20%}"

mkdir -p "$WORK"/{ne,gn,out}
fetch() { [ -s "$2" ] || curl -fsSL --retry 3 -o "$2" "$1"; }

echo "downloading sources into $WORK"
fetch "$NE/ne_10m_admin_0_countries.geojson" "$WORK/ne/countries.geojson"
fetch "$NE/ne_10m_admin_1_states_provinces.geojson" "$WORK/ne/admin1.geojson"
fetch "$GN/cities500.zip" "$WORK/gn/cities500.zip"
fetch "$GN/JP.zip" "$WORK/gn/JP.zip"
fetch "$GN/alternatenames/JP.zip" "$WORK/gn/alternatenames-JP.zip"
fetch "$GN/alternateNamesV2.zip" "$WORK/gn/alternateNamesV2.zip"
fetch "$GN/admin1CodesASCII.txt" "$WORK/gn/admin1CodesASCII.txt"
fetch "$GN/countryInfo.txt" "$WORK/gn/countryInfo.txt"

echo "simplifying boundaries"
# Countries: one feature per ISO code (the largest part names the group).
$MAPSHAPER "$WORK/ne/countries.geojson" \
  -filter 'TYPE != "Indeterminate" && TYPE != "Lease"' \
  -each 'id = (ISO_A2_EH && ISO_A2_EH != "-99") ? ISO_A2_EH : ADM0_A3; name = NAME_EN || NAME; name_ja = NAME_JA || ""; a3 = ADM0_A3' \
  -filter-fields id,name,name_ja,a3 \
  -simplify weighted "$KEEP" keep-shapes \
  -clean \
  -sort 'this.area' descending \
  -dissolve id copy-fields=name,name_ja,a3 \
  -o "$WORK/out/countries.ndjson" format=geojson ndjson precision=0.001

# First-level subdivisions: ISO 3166-2 code where Natural Earth has a proper
# one, its own adm1_code otherwise.
$MAPSHAPER "$WORK/ne/admin1.geojson" \
  -each 'var iso = iso_3166_2 || ""; id = /^[A-Z]{2}-[A-Z0-9]{1,3}$/.test(iso) ? iso : adm1_code; cc = (iso_a2 && iso_a2 != "-99") ? iso_a2 : ""; a3 = adm0_a3; gn = gn_a1_code || ""; name = name_en || name || ""; name_ja = name_ja || ""' \
  -filter-fields id,name,name_ja,cc,a3,gn \
  -simplify weighted "$KEEP" keep-shapes \
  -clean \
  -sort 'this.area' descending \
  -dissolve id copy-fields=name,name_ja,cc,a3,gn \
  -o "$WORK/out/admin1.ndjson" format=geojson ndjson precision=0.001

echo "building data files"
python3 -I "$HERE/build_data.py" "$WORK" "$HERE"
ls -la "$HERE"
