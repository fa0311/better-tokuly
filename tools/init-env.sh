#!/usr/bin/env bash
# Writes `.env` for `docker compose` from `.env.example`, filling in freshly
# generated secrets. Run once before the first `docker compose up`.
#
#   tools/init-env.sh           # refuses to touch an existing .env
#   tools/init-env.sh --force   # start over (existing data volumes will not accept the new secrets)
set -euo pipefail
cd "$(dirname "$0")/.."

if [ -e .env ] && [ "${1:-}" != "--force" ]; then
  echo ".env already exists; leaving it alone (use --force to replace it)" >&2
  exit 1
fi

# 32 random bytes as 64 hex characters, from whatever this machine has.
secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
  fi
}

umask 077
{
  echo "# Tokuly deployment settings (created by tools/init-env.sh). See .env.example for what each one does."
  # Skip the template's own header (up to its first blank line), then fill in every empty secret.
  sed '1,/^$/d' .env.example | while IFS= read -r line; do
    case "$line" in
      TOKULY_SECRET= | POSTGRES_PASSWORD= | STORAGE_SECRET_KEY= | CLICKHOUSE_PASSWORD= | MEILI_MASTER_KEY= | LIVEKIT_API_SECRET=)
        line="$line$(secret)"
        ;;
    esac
    printf '%s\n' "$line"
  done
} >.env

echo "Wrote .env with new secrets. Next: docker compose up -d, then open http://localhost"
