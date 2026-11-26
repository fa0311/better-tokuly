#!/usr/bin/env bash
# Backs up the Compose stack of this checkout into one dated directory
# (docs/backup.md): the database, the stored files, the event streams, Valkey,
# ClickHouse, `.env` and `config/`. The search index is not kept: restore.sh
# rebuilds it from the database.
#
#   tools/backup.sh [--dir <dir>] [--keep <n>] [--no-analytics]
#
#   --dir <dir>      where the dated directories go (default: backups/)
#   --keep <n>       afterwards, delete all but the newest <n> backups in <dir>
#   --no-analytics   leave out ClickHouse (analytics only; it starts empty after a restore)
#
# The backend and everything it writes to stop while the data is copied, so the
# copies agree with each other; what was running is started again at the end,
# also when the backup fails. The directory holds every secret of the
# deployment: keep it where only the operator can read it.
set -euo pipefail
cd "$(dirname "$0")/.."

root=backups keep="" analytics=yes
while [ $# -gt 0 ]; do
  case "$1" in
    --dir) root="${2:?--dir needs a directory}"; shift 2 ;;
    --keep) keep="${2:?--keep needs a number}"; shift 2 ;;
    --no-analytics) analytics=no; shift ;;
    *) echo "usage: tools/backup.sh [--dir <dir>] [--keep <n>] [--no-analytics]" >&2; exit 2 ;;
  esac
done
if [ -n "$keep" ] && ! [[ "$keep" =~ ^[1-9][0-9]*$ ]]; then echo "--keep needs a number above 0" >&2; exit 2; fi
[ -e .env ] || { echo "no .env here: is this the directory the stack runs from?" >&2; exit 1; }

# The volumes copied as they are, with the services stopped. Postgres is dumped
# instead, which a newer PostgreSQL can read too.
volumes=(storage-data nats-data valkey-data edge-data)
[ "$analytics" = yes ] && volumes+=(clickhouse-data)

project="${COMPOSE_PROJECT_NAME:-$(docker compose config | sed -n 's/^name: //p')}"
umask 077
dir="$root/$(date +%Y-%m-%dT%H%M%S)"
mkdir -p "$dir"
echo "Backing up to $dir"

# Everything that runs is stopped except the database; the trap starts it again.
running="$(docker compose ps --services --status running)"
restart() { [ -z "$running" ] || docker compose start $running >/dev/null; }
trap restart EXIT
docker compose up -d --wait postgres >/dev/null
stopping="$(grep -vx postgres <<<"$running" || true)"
[ -z "$stopping" ] || docker compose stop $stopping

docker compose exec -T postgres pg_dump -U tokuly -d tokuly --format=custom >"$dir/postgres.dump"
echo "  postgres.dump ($(du -h "$dir/postgres.dump" | cut -f1))"

# A container of the database image (it has tar) reads each volume.
image="$(docker inspect --format '{{.Config.Image}}' "$(docker compose ps -q postgres)")"
for volume in "${volumes[@]}"; do
  if ! docker volume inspect "${project}_$volume" >/dev/null 2>&1; then
    echo "  $volume: none yet, skipped"; continue
  fi
  docker run --rm -v "${project}_$volume:/volume:ro" "$image" tar -C /volume -cf - . >"$dir/$volume.tar"
  echo "  $volume.tar ($(du -h "$dir/$volume.tar" | cut -f1))"
done

cp .env "$dir/env"
[ -d config ] && cp -R config "$dir/config"
# Which images wrote this, to restore with the same version.
docker compose images >"$dir/images.txt"
echo "  env, config/, images.txt"

restart; trap - EXIT
echo "Done. $dir holds every secret of this deployment (.env): keep it private, and encrypt it before it leaves this machine."

if [ -n "$keep" ]; then
  # The dated names sort by time; everything but the newest $keep goes.
  ls -1d "$root"/????-??-??T?????? 2>/dev/null | head -n -"$keep" | while read -r old; do
    rm -rf "$old"; echo "Removed $old"
  done
fi
