#!/usr/bin/env bash
# Writes `.env` for `docker compose` from `.env.example`, filling in freshly
# generated secrets, and a first `config/tokuly.toml` when there is none. Run
# once before the first `docker compose up`, and again after an update that
# needs a secret your `.env` does not have yet.
#
# `.env` holds only secrets and how the deployment is reached; every other
# setting belongs in config/tokuly.toml (see tokuly.example.toml) or the staff
# console.
#
#   tools/init-env.sh           # no .env: write one. An existing .env: add the secrets it lacks, touch nothing else
#   tools/init-env.sh --force   # start over (existing data volumes will not accept the new secrets)
set -euo pipefail
cd "$(dirname "$0")/.."

# The secrets this script generates: the lines of `.env.example` that name one of them and end at `=`.
SECRETS=(TOKULY_SECRET POSTGRES_PASSWORD STORAGE_SECRET_KEY CLICKHOUSE_PASSWORD MEILI_MASTER_KEY LIVEKIT_API_SECRET TOKULY_LIVE_HOOK_SECRET TOKULY_MEDIAMTX_API_SECRET)

# 32 random bytes as 64 hex characters, from whatever this machine has.
secret() {
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    od -An -N32 -tx1 /dev/urandom | tr -d ' \n'
  fi
}

is_secret() {
  local name
  for name in "${SECRETS[@]}"; do [ "$name" = "$1" ] && return 0; done
  return 1
}

# The declarative settings: a short start, never overwritten.
write_config() {
  [ -e config/tokuly.toml ] && return 0
  mkdir -p config
  cat >config/tokuly.toml <<'TOML'
# Tokuly settings (created by tools/init-env.sh). Every key, its default and
# what it does: tokuly.example.toml. Unknown keys stop the backend, so typos
# show at once. Restart what a change needs: `docker compose restart api worker`.
# Settings set here are locked in the staff console (「サーバー設定」); the
# ones you leave out can be changed there.

[identity]
# Set to false to stop new sign-ups once everyone is in.
registration_open = true

[wallet]
# No payment processor is bundled. With this on, "buying" coins is free, which
# is what a private deployment among friends wants. Turn it off when you plug
# in a real payment provider.
dev_provider = true
TOML
  chmod 644 config/tokuly.toml
  echo "Wrote config/tokuly.toml (settings other than secrets go there)."
}
write_config

umask 077

if [ -e .env ] && [ "${1:-}" != "--force" ]; then
  # An existing deployment: its secrets are what its data was created with, so
  # nothing in the file is changed. Only secrets that are not set at all are added.
  missing=()
  for name in "${SECRETS[@]}"; do
    grep -Eq "^[[:space:]]*(export[[:space:]]+)?$name=.+" .env || missing+=("$name")
  done
  if [ "${#missing[@]}" -eq 0 ]; then
    echo ".env already has every secret; nothing to do (use --force to start over)"
    exit 0
  fi
  {
    # The file may not end with a newline.
    [ -z "$(tail -c1 .env)" ] || echo
    echo
    echo "# Added by tools/init-env.sh on $(date +%Y-%m-%d): secrets this deployment did not have yet."
    for name in "${missing[@]}"; do printf '%s=%s\n' "$name" "$(secret)"; done
  } >>.env
  echo "Added to .env: ${missing[*]}. Everything else was left as it was. Next: docker compose up -d"
  exit 0
fi

{
  echo "# Tokuly deployment settings (created by tools/init-env.sh). See .env.example for what each one does."
  # Skip the template's own header (up to its first blank line), then fill in every empty secret.
  sed '1,/^$/d' .env.example | while IFS= read -r line; do
    case "$line" in
      *=) if is_secret "${line%=}"; then line="$line$(secret)"; fi ;;
    esac
    printf '%s\n' "$line"
  done
} >.env

echo "Wrote .env with new secrets. Next: docker compose up -d, then open http://localhost"
