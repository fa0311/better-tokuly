#!/usr/bin/env bash
# The previous release's backend keeps working on this checkout's schema, as it has
# to while the API and the workers are replaced one after the other (backend/README.md,
# "After a release"). Next to a Compose stack of this checkout's images (CI only: it
# empties the stack), twice:
#
#   - an empty database, migrated by this checkout's image;
#   - the previous release's demo data (tools/ci/demo-data.sh), migrated by this
#     checkout's image: a migration that fails on existing rows fails here;
#
# then the previous release's api, realtime and worker take over and
# tools/ci/smoke.sh runs. The previous release is the highest stable tag not on
# this commit; without one there is nothing to check and it says so.
#
#   tools/ci/previous-release.sh --destroy-this-stack [<origin>]   default http://localhost
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "${1:-}" = --destroy-this-stack ] || { echo "usage: tools/ci/previous-release.sh --destroy-this-stack [<origin>]" >&2; exit 2; }
ORIGIN="${2:-http://localhost}"
IMAGES="${TOKULY_IMAGES:-ghcr.io/fa0311/better-tokuly}"

release="$(git tag --list 'v[0-9]*' --no-contains HEAD | grep -v -- - | sort -V | tail -1 || true)"
if [ -z "$release" ]; then
  echo "No stable release yet: no previous backend to check."
  exit 0
fi
echo "Previous release: $release"
TOKULY_IMAGES="$IMAGES" TOKULY_VERSION=previous tools/ci/use-images.sh backend="$IMAGES/backend:${release#v}"

WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

# This checkout's migration has run; the previous release's backend replaces this one's.
previous_backend_smoke() {
  echo "== $release's api, realtime and worker"
  TOKULY_VERSION=previous docker compose up -d --no-deps --wait --wait-timeout 300 api realtime worker
  tools/ci/smoke.sh "$ORIGIN"
}

echo "== An empty database"
docker compose down -v
tools/ci/stack-up.sh
previous_backend_smoke

echo "== $release's demo data"
if tools/ci/demo-data.sh fetch "$release" "$WORK/demo.dump"; then
  docker compose down -v
  docker compose up -d --wait postgres
  docker compose exec -T postgres pg_restore -U tokuly -d tokuly --no-owner --exit-on-error <"$WORK/demo.dump"
  tools/ci/stack-up.sh
  previous_backend_smoke
else
  echo "::warning::$release has no demo data: only the empty database was checked."
fi
echo "$release's backend works on this checkout's schema."
