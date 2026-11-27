#!/usr/bin/env bash
# Upgrades the schema of the latest stable release to this checkout's, the way
# `tokuly migrate` does: the release's files in order, then the files added
# since (each in its own transaction, except those starting with
# `-- no-transaction`, which are sent as one statement outside one). The
# upgraded schema must equal a fresh install's. The release is the highest
# stable tag that is not on this commit, so the commit being tagged is checked
# against the release before it. Without one there is nothing to upgrade from
# (alpha upgrades are not supported) and it says so.
#
# The release's schema comes with rows: its demo data (tools/ci/demo-data.sh, the
# release's own unless a file is given), so that a change that only fails on a
# filled table (a NOT NULL column without a DEFAULT) fails here. A release without
# demo data is upgraded from its files and empty tables, with a warning.
#
#   tools/ci/migrations-upgrade.sh <postgres URL> [<demo data>]   a server it may create and drop databases on
set -euo pipefail
cd "$(dirname "$0")/../.."
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: tools/ci/migrations-upgrade.sh <postgres URL> [<demo data>]" >&2; exit 2; }
SERVER="$1" DEMO="${2:-}"
DIR=backend/migrations
export PGOPTIONS="-c client_min_messages=warning"

# sqlx knows a migration by its number: a second file with the same one fails on every database.
shared="$(git ls-files "$DIR/*.sql" | sed 's|.*/\([0-9]*\)_.*|\1|' | sort | uniq -d)"
[ -z "$shared" ] || { echo "More than one migration file has the number: ${shared//$'\n'/ }" >&2; exit 1; }

release="$(git tag --list 'v[0-9]*' --no-contains HEAD | grep -v -- - | sort -V | tail -1 || true)"
if [ -z "$release" ]; then
  echo "No stable release yet: nothing to upgrade from."
  exit 0
fi
echo "Upgrading from $release"

mapfile -t released < <(git ls-tree --name-only "$release" "$DIR/" | grep '\.sql$' | sort)
mapfile -t current < <(git ls-files "$DIR/*.sql" | sort)
mapfile -t added < <(comm -13 <(printf '%s\n' "${released[@]}") <(printf '%s\n' "${current[@]}"))
git diff --quiet "$release" -- "${released[@]}" || { echo "Released migrations were edited since $release:" >&2; git diff --stat "$release" -- "${released[@]}" >&2; exit 1; }

WORK="$(mktemp -d)"
database() { printf '%s/%s' "${SERVER%/*}" "$1"; }
cleanup() { for db in upgraded fresh; do psql -q "$SERVER" -c "DROP DATABASE IF EXISTS tokuly_migrations_$db" >/dev/null || true; done; rm -rf "$WORK"; }
trap cleanup EXIT

# apply <database> <file>…
apply() {
  local url; url="$(database "$1")"; shift
  for file in "$@"; do
    if head -n 1 "$file" | grep -q '^-- no-transaction'; then
      psql -q -X -v ON_ERROR_STOP=1 "$url" -c "$(cat "$file")" || { echo "FAILED: $file" >&2; exit 1; }
    else
      psql -q -X -v ON_ERROR_STOP=1 -1 "$url" -f "$file" || { echo "FAILED: $file" >&2; exit 1; }
    fi
  done
}

for db in upgraded fresh; do
  psql -q "$SERVER" -c "DROP DATABASE IF EXISTS tokuly_migrations_$db" -c "CREATE DATABASE tokuly_migrations_$db"
done
if [ -z "$DEMO" ] && tools/ci/demo-data.sh fetch "$release" "$WORK/demo.dump"; then DEMO="$WORK/demo.dump"; fi
echo "== $release"
apply tokuly_migrations_upgraded "${released[@]}"
if [ -n "$DEMO" ]; then
  echo "== its demo data"
  # The data must be of the release: exactly its files applied, no more.
  applied="$(pg_restore --data-only --table=_sqlx_migrations -f - "$DEMO" | sed -n '/^COPY /,/^\\\.$/p' | sed '1d;$d' | cut -f1 | sort -n)"
  expected="$(printf '%s\n' "${released[@]##*/}" | sed 's/_.*//; s/^0*//' | sort -n)"
  [ "$applied" = "$expected" ] || { echo "The demo data is not of $release's migrations:" >&2; diff <(echo "$expected") <(echo "$applied") >&2; exit 1; }
  # What the files put in the tables makes way for the rows of the demo (sqlx's own table
  # is not in an install made with psql). The schema stays the files': a restored one
  # prints some constraints differently.
  psql -q -X -v ON_ERROR_STOP=1 "$(database tokuly_migrations_upgraded)" <<'SQL'
SELECT 'TRUNCATE ' || string_agg(format('%I.%I', n.nspname, c.relname), ', ') || ' CASCADE'
FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p') AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND NOT EXISTS (SELECT FROM pg_depend d WHERE d.classid = 'pg_class'::regclass AND d.objid = c.oid AND d.deptype = 'e')
\gexec
SQL
  pg_restore --data-only --disable-triggers --no-owner --exit-on-error -d "$(database tokuly_migrations_upgraded)" \
    -L <(pg_restore -l "$DEMO" | grep -v ' _sqlx_migrations ') "$DEMO"
else
  echo "::warning::No demo data of $release: the upgrade runs on empty tables."
fi
echo "== added since: ${added[*]:-none}"
apply tokuly_migrations_upgraded "${added[@]}"
echo "== fresh install"
apply tokuly_migrations_fresh "${current[@]}"

for db in upgraded fresh; do
  pg_dump --schema-only --no-owner "$(database "tokuly_migrations_$db")" | grep -v '^\\\(un\)\?restrict ' > "$WORK/$db.sql"
done
diff -u "$WORK/fresh.sql" "$WORK/upgraded.sql" || { echo "The upgraded schema differs from a fresh install's (above)." >&2; exit 1; }
echo "The schema of $release upgrades to this checkout's."
