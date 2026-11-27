#!/usr/bin/env bash
# The database of a release filled with demo data: what the upgrade checks of the
# next release start from (tools/ci/migrations-upgrade.sh, tools/ci/previous-release.sh),
# so that a migration that only fails on existing rows fails there.
#
#   tools/ci/demo-data.sh make <file> [<origin>] [<seconds>]   default http://localhost, 240
#   tools/ci/demo-data.sh fetch <release tag> <file>
#
# `make` empties the Compose stack it runs next to (CI only), brings it up with the
# demo bots, lets them play for <seconds>, runs tools/ci/smoke.sh once and writes the
# database as a `pg_dump --format=custom` file. The Release workflow publishes it as the
# image $TOKULY_IMAGES/ci-demo-data:<version> (the file is /demo.dump).
# `fetch` writes that file of a release; it fails, saying so, when there is none.
set -euo pipefail
cd "$(dirname "$0")/../.."
IMAGES="${TOKULY_IMAGES:-ghcr.io/fa0311/better-tokuly}"
usage() { echo "usage: tools/ci/demo-data.sh make <file> [<origin>] [<seconds>] | fetch <release tag> <file>" >&2; exit 2; }

case "${1:-}" in
  make)
    [ $# -ge 2 ] || usage
    out="$2" origin="${3:-http://localhost}" seconds="${4:-240}"
    tools/demo-env.sh
    docker compose down -v
    tools/ci/stack-up.sh
    echo "The bots play for ${seconds}s"
    sleep "$seconds"
    docker compose stop demo
    tools/ci/smoke.sh "$origin"
    sql() { docker compose exec -T postgres psql -U tokuly -d tokuly -Atc "$1"; }
    echo "users $(sql 'SELECT count(*) FROM users'), friendships $(sql 'SELECT count(*) FROM friendships'), notes $(sql 'SELECT count(*) FROM notes')"
    [ "$(sql 'SELECT count(*) FROM users')" -gt 10 ] || { echo "FAILED: the demo made no accounts" >&2; exit 1; }
    docker compose exec -T postgres pg_dump -U tokuly --format=custom tokuly >"$out"
    ls -la "$out"
    ;;
  fetch)
    [ $# -eq 3 ] || usage
    image="$IMAGES/ci-demo-data:${2#v}"
    if ! docker image inspect "$image" >/dev/null 2>&1 && ! docker pull -q "$image" >/dev/null 2>&1; then
      echo "No demo data of $2 ($image)." >&2
      exit 1
    fi
    container="$(docker create "$image" none)"
    docker cp -q "$container:/demo.dump" "$3"
    docker rm "$container" >/dev/null
    ;;
  *) usage ;;
esac
