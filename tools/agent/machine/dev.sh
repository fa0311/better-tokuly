#!/usr/bin/env bash
# Sandbox helper: run the backend binary and the web dev server on sandbox ports.
set -euo pipefail
REPO=${TOKULY_REPO:-$(git -C "$(dirname "$0")" rev-parse --show-toplevel)}
RUN=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/run; LOGS=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/logs
BIN=${TOKULY_BIN:-${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/target/main/debug/tokuly}; [ -x "$BIN" ] || BIN=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/bin/tokuly-dev-api
stop_one() { [ -f "$RUN/$1.pid" ] && kill "$(cat "$RUN/$1.pid")" 2>/dev/null || true; rm -f "$RUN/$1.pid"; }
case "${1:-}" in
  api)
    stop_one api
    eval "$("$REPO/tools/dev/infra.sh" env)"
    # `dev.sh api fresh` recreates the database (needed when a migration that already ran was edited);
    # plain `dev.sh api` keeps it and only applies new migrations, so running browser tests survive a restart.
    ADMIN_URL="$TOKULY_DATABASE_URL"
    if [ "${2:-}" = fresh ]; then
      psql "$ADMIN_URL" -qc 'DROP DATABASE IF EXISTS tokuly_dev WITH (FORCE)' >/dev/null
    fi
    psql "$ADMIN_URL" -qAtc "SELECT 1 FROM pg_database WHERE datname='tokuly_dev'" | grep -q 1 \
      || psql "$ADMIN_URL" -qc 'CREATE DATABASE tokuly_dev' >/dev/null
    export TOKULY_DATABASE_URL="${ADMIN_URL%/*}/tokuly_dev"
    export TOKULY_PUBLIC_URL=http://localhost:53000 TOKULY_API_LISTEN=127.0.0.1:58081
    # The account with this handle becomes admin when it signs up (used by the browser tests).
    # Browser tests sign up many accounts from one address.
    export TOKULY_ADMIN_USER=e2e-admin@e2e.tokuly.test TOKULY_RATE_LIMITS=false TOKULY_NEWS_ALLOW_LOOPBACK=true
    export TOKULY_RT_WS_LISTEN=127.0.0.1:58082 TOKULY_RT_WT_LISTEN=0.0.0.0:54433
    export TOKULY_RT_PUBLIC_WT_URL=https://127.0.0.1:54433/rt TOKULY_RT_PUBLIC_WS_URL=ws://localhost:58082/rt/ws
    # Web engineers in their own checkouts run dev servers on 53010-53039 against this API.
    export TOKULY_RT_ALLOWED_ORIGINS="$(for p in $(seq 53010 53039); do printf 'http://localhost:%s,' "$p"; done | sed 's/,$//')"
    nohup "$BIN" all >"$LOGS/tokuly.log" 2>&1 & echo $! >"$RUN/api.pid" ;;
  web)
    stop_one web
    cd "$REPO/apps/web"
    TOKULY_DEV_API_URL=http://127.0.0.1:58081 TOKULY_DEV_TILES_URL=http://127.0.0.1:58090 \
      TOKULY_DEV_MEDIA_URL=http://127.0.0.1:59000/tokuly-media \
      nohup pnpm exec vite dev --port 53000 --host 127.0.0.1 >"$LOGS/web.log" 2>&1 & echo $! >"$RUN/web.pid" ;;
  api-here)
    # A complete, separate backend for the checkout you are in: its own database, cache, event
    # streams, analytics database and search indexes, so it can run next to the shared API.
    #   TOKULY_BIN=<your build>/debug/tokuly dev.sh api-here <name> <api port> <web port>
    # Listens on <api port> (API), <api port>+1 (realtime WebSocket), <api port>+2 (WebTransport).
    # The database is recreated on every start. logs: $TOKULY_DEV_HOME/logs/api-<name>.log
    name=${2:?name}; port=${3:?api port}; webport=${4:?web port}
    [ -n "${TOKULY_BIN:-}" ] || { echo "set TOKULY_BIN to the tokuly binary of your build"; exit 1; }
    stop_one "api-$name"
    eval "$("$REPO/tools/dev/infra.sh" env)"
    ADMIN_URL="$TOKULY_DATABASE_URL"; db="tokuly_${name//[^a-z0-9_]/_}"
    psql "$ADMIN_URL" -qc "DROP DATABASE IF EXISTS $db WITH (FORCE)" -c "CREATE DATABASE $db" >/dev/null
    curl -fsS "$TOKULY_CLICKHOUSE_URL/" --user "$TOKULY_CLICKHOUSE_USER:$TOKULY_CLICKHOUSE_PASSWORD" \
      --data-binary "CREATE DATABASE IF NOT EXISTS $db" >/dev/null
    export TOKULY_DATABASE_URL="${ADMIN_URL%/*}/$db" TOKULY_CLICKHOUSE_DATABASE="$db"
    export TOKULY_VALKEY_URL="${TOKULY_VALKEY_URL%/}/$(( port % 14 + 2 ))"
    export TOKULY_NATS_PREFIX="${name//[^a-zA-Z0-9_]/_}" TOKULY_SEARCH_INDEX_PREFIX="${name//[^a-zA-Z0-9_]/_}_"
    export TOKULY_PUBLIC_URL="http://localhost:$webport" TOKULY_API_LISTEN="127.0.0.1:$port"
    export TOKULY_ADMIN_USER=e2e-admin@e2e.tokuly.test TOKULY_RATE_LIMITS=false TOKULY_NEWS_ALLOW_LOOPBACK=true
    export TOKULY_RT_WS_LISTEN="127.0.0.1:$((port + 1))" TOKULY_RT_WT_LISTEN="0.0.0.0:$((port + 2))"
    export TOKULY_RT_PUBLIC_WT_URL="https://127.0.0.1:$((port + 2))/rt" TOKULY_RT_PUBLIC_WS_URL="ws://localhost:$((port + 1))/rt/ws"
    nohup "$TOKULY_BIN" all >"$LOGS/api-$name.log" 2>&1 & echo $! >"$RUN/api-$name.pid" ;;
  api-here-stop) stop_one "api-${2:?name}" ;;
  web-here)
    # A dev server for the checkout you are in (a git worktree):
    #   dev.sh web-here <name> <port> [<api port>]     default API: the shared one on 58081
    # logs: $TOKULY_DEV_HOME/logs/web-<name>.log
    name=${2:?name}; port=${3:?port}; api=${4:-58081}
    stop_one "web-$name"
    cd "$(git rev-parse --show-toplevel)/apps/web"
    TOKULY_DEV_API_URL=http://127.0.0.1:$api TOKULY_DEV_TILES_URL=http://127.0.0.1:58090 \
      TOKULY_DEV_MEDIA_URL=http://127.0.0.1:59000/tokuly-media \
      nohup pnpm exec vite dev --port "$port" --strictPort --host 127.0.0.1 >"$LOGS/web-$name.log" 2>&1 & echo $! >"$RUN/web-$name.pid" ;;
  web-here-stop) stop_one "web-${2:?name}" ;;
  stop) stop_one api; stop_one web ;;
  *) echo "usage: dev.sh api [fresh] | web | api-here <name> <api port> <web port> | api-here-stop <name> | web-here <name> <port> [<api port>] | web-here-stop <name> | stop"; exit 1 ;;
esac
