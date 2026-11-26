#!/usr/bin/env bash
# The stack the end-to-end tests run against, without Docker (GitHub's macOS
# runners have none): the backing services and the edge of tools/dev/infra.sh,
# and the backend as `tokuly migrate` then `tokuly all` (API, realtime gateway
# and workers in one process). The settings are the ones the Compose stack is
# tested with in e2e.yml: production, no request limits (the tests make their
# accounts from one address), mail caught by Mailpit.
#
#   tools/ci/native-stack.sh <tokuly binary> [<timeout seconds>]   default 300
#
# Everything listens on this machine; the edge is http://localhost:$P_EDGE
# (8080). Logs: $TOKULY_DEV_HOME/logs (tokuly.log for the backend).
set -euo pipefail
cd "$(dirname "$0")/../.."
tokuly="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
timeout="${2:-300}"
logs="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/logs"
run="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/run"
: "${P_EDGE:=8080}" "${P_API:=8081}" "${P_RT_WS:=8082}" "${P_S3:=9000}" "${P_LIVEKIT:=7880}"
export P_EDGE P_API P_RT_WS P_S3 P_LIVEKIT

TOKULY_DEV_EDGE=1 tools/dev/infra.sh up
eval "$(tools/dev/infra.sh env)"
# config/tokuly.toml as Compose mounts it (and a .env for its secret).
tools/init-env.sh >/dev/null
TOKULY_SECRET="$(sed -n 's/^TOKULY_SECRET=//p' .env)"

export TOKULY_ENV=production
export TOKULY_CONFIG="$PWD/config/tokuly.toml"
export TOKULY_SECRET
export TOKULY_PUBLIC_URL="http://localhost:$P_EDGE"
export TOKULY_RATE_LIMITS=false
export TOKULY_API_LISTEN="127.0.0.1:$P_API"
export TOKULY_RT_WS_LISTEN="127.0.0.1:$P_RT_WS"
# What the apps are handed, as in compose.yaml: this machine by name.
export TOKULY_S3_PUBLIC_ENDPOINT="http://localhost:$P_S3"
export TOKULY_LIVEKIT_PUBLIC_URL="ws://localhost:$P_LIVEKIT"

"$tokuly" migrate >"$logs/migrate.log" 2>&1 || { tail -50 "$logs/migrate.log"; echo "native-stack: migrate failed" >&2; exit 1; }
nohup "$tokuly" all >"$logs/tokuly.log" 2>&1 &
echo $! >"$run/tokuly.pid"

# Ready when the API answers through the edge.
deadline=$((SECONDS + timeout))
until curl -fsS -o /dev/null "http://127.0.0.1:$P_EDGE/api/readyz"; do
  if ! kill -0 "$(cat "$run/tokuly.pid")" 2>/dev/null; then
    tail -50 "$logs/tokuly.log"; echo "native-stack: tokuly exited" >&2; exit 1
  fi
  if [ "$SECONDS" -ge "$deadline" ]; then
    tail -50 "$logs/tokuly.log"; echo "native-stack: not ready after ${timeout}s" >&2; exit 1
  fi
  sleep 2
done
tools/dev/infra.sh status
echo "native-stack: up at http://localhost:$P_EDGE"
