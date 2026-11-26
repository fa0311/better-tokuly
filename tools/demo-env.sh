#!/usr/bin/env bash
# Turns the demo on for `docker compose`: adds to `.env` the Compose profile
# and what the demo simulator needs (see README.md, 「デモを動かす」).
#
#   COMPOSE_PROFILES            gets `demo`
#   TOKULY_DEV_DEMO_OPERATORS   addresses of the accounts that own the bots (random, so nobody
#                               can sign up with them first); the server and the simulator read it
#   TOKULY_DEMO_PASSWORD        their password
#
# Run it after tools/init-env.sh, then `docker compose up -d`. Running it again changes nothing.
# To turn the demo off, take `demo` out of COMPOSE_PROFILES (the bots stay as they are) and,
# to take the bots' rights away as well, remove TOKULY_DEV_DEMO_OPERATORS.
set -euo pipefail
cd "$(dirname "$0")/.."
[ -e .env ] || { echo "No .env yet: run tools/init-env.sh first." >&2; exit 1; }

random() {  # <bytes>: that many random bytes as hex
  if command -v openssl >/dev/null 2>&1; then openssl rand -hex "$1"; else od -An -N"$1" -tx1 /dev/urandom | tr -d ' \n'; fi
}
has() { grep -Eq "^[[:space:]]*(export[[:space:]]+)?$1=.+" .env; }

added=()
umask 077
# The file may not end with a newline.
[ -z "$(tail -c1 .env)" ] || echo >>.env
if ! has TOKULY_DEV_DEMO_OPERATORS; then
  tag=$(random 6)
  printf 'TOKULY_DEV_DEMO_OPERATORS=%s\n' "demo-$tag-1@demo.tokuly.invalid,demo-$tag-2@demo.tokuly.invalid,demo-$tag-3@demo.tokuly.invalid" >>.env
  added+=(TOKULY_DEV_DEMO_OPERATORS)
fi
if ! has TOKULY_DEMO_PASSWORD; then
  printf 'TOKULY_DEMO_PASSWORD=%s\n' "$(random 24)" >>.env
  added+=(TOKULY_DEMO_PASSWORD)
fi
if grep -Eq '^COMPOSE_PROFILES=[[:space:]]*$' .env; then
  # The empty line of the template: filled in where it stands.
  sed -i.bak -E 's/^COMPOSE_PROFILES=[[:space:]]*$/COMPOSE_PROFILES=demo/' .env && rm -f .env.bak
  added+=(COMPOSE_PROFILES)
elif ! has COMPOSE_PROFILES; then
  echo 'COMPOSE_PROFILES=demo' >>.env
  added+=(COMPOSE_PROFILES)
elif ! grep -Eq '^[[:space:]]*(export[[:space:]]+)?COMPOSE_PROFILES=.*\bdemo\b' .env; then
  echo "COMPOSE_PROFILES is already set in .env: add 'demo' to it yourself (comma-separated)." >&2
fi

if [ "${#added[@]}" -eq 0 ]; then
  echo ".env already has the demo's settings; nothing to do."
else
  echo "Added to .env: ${added[*]}. Next: docker compose up -d (the api and worker restart with the new setting)."
fi
