#!/usr/bin/env bash
# Brings back a backup made by tools/backup.sh (docs/backup.md) into the
# Compose stack of this checkout, then starts it. The search index is rebuilt
# from the database while the stack runs.
#
#   tools/restore.sh <backup dir>              into a stack without data (a new machine)
#   tools/restore.sh <backup dir> --replace    throw away this stack's data first
#
# Without --replace it stops if this stack already has data, or a .env or
# config/tokuly.toml that differs from the backup's: the backed-up data can
# only be read with the backup's secrets. --replace keeps the old two as
# `.env.before-restore` and `config.before-restore/`.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() { echo "usage: tools/restore.sh <backup dir> [--replace]" >&2; exit 2; }
[ $# -ge 1 ] && [ $# -le 2 ] || usage
backup="$1" replace=no
case "${2:-}" in "") ;; --replace) replace=yes ;; *) usage ;; esac
[ -f "$backup/postgres.dump" ] && [ -f "$backup/env" ] || { echo "$backup is not a backup of tools/backup.sh" >&2; exit 1; }

# Every volume with data of the deployment. The search index is among them so
# that a replaced stack does not keep an index of the old data.
volumes=(postgres-data storage-data nats-data valkey-data edge-data clickhouse-data search-data)

# Named as the backup's settings name it (there may be no .env here yet).
project="${COMPOSE_PROJECT_NAME:-$(docker compose --env-file "$backup/env" config | sed -n 's/^name: //p')}"
existing=()
for volume in "${volumes[@]}"; do
  docker volume inspect "${project}_$volume" >/dev/null 2>&1 && existing+=("$volume")
done

# Settings first: the secrets in .env are the ones the data was written with.
differs() { [ -e "$1" ] && ! diff -rq "$1" "$2" >/dev/null 2>&1; }
config_differs=no
[ -d "$backup/config" ] && differs config "$backup/config" && config_differs=yes
if [ "$replace" = no ]; then
  [ ${#existing[@]} -eq 0 ] || { echo "this stack already has data (${existing[*]}); add --replace to throw it away" >&2; exit 1; }
  differs .env "$backup/env" && { echo ".env differs from the backup's; add --replace to use the backup's" >&2; exit 1; }
  [ "$config_differs" = no ] || { echo "config/ differs from the backup's; add --replace to use the backup's" >&2; exit 1; }
fi
differs .env "$backup/env" && mv .env .env.before-restore
cp "$backup/env" .env
if [ -d "$backup/config" ]; then
  [ "$config_differs" = yes ] && rm -rf config.before-restore && mv config config.before-restore
  mkdir -p config && cp -R "$backup/config/." config/
  # Read by the containers' own user (the backup itself is private).
  chmod 755 config && find config -type f -exec chmod 644 {} +
fi

docker compose down
if [ ${#existing[@]} -gt 0 ]; then
  for volume in "${existing[@]}"; do docker volume rm "${project}_$volume" >/dev/null; done
  echo "Removed the volumes ${existing[*]}"
fi

# Creates the volumes (and the containers, not started).
docker compose up --no-start
image="$(docker inspect --format '{{.Config.Image}}' "$(docker compose ps -aq postgres)")"
for volume in "${volumes[@]}"; do
  [ -f "$backup/$volume.tar" ] || continue
  docker run --rm -i -v "${project}_$volume:/volume" "$image" tar -C /volume -xf - <"$backup/$volume.tar"
  echo "  $volume restored"
done

# The database goes back before the migration runs: it is the one that ran then.
docker compose up -d --wait postgres
docker compose exec -T postgres pg_restore -U tokuly -d tokuly --no-owner --exit-on-error <"$backup/postgres.dump"
echo "  postgres restored"
# The search index starts empty: have every document written to it again
# (what 「再インデックス」 in the staff console does, for every kind at once).
docker compose exec -T postgres psql -U tokuly -d tokuly -qc \
  "UPDATE search_documents SET generation = generation + 1, queued_at = now() WHERE doc IS NOT NULL"

docker compose up -d
echo "Done. The stack is starting; search results come back while the worker rebuilds the index."
