#!/usr/bin/env bash
# Sandbox helper: run the backend binary and the web dev server on sandbox ports.
: "${TOKULY_DEV_HOME:=$HOME/tokuly-dev}"
set -euo pipefail
REPO=${TOKULY_REPO:-$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)}
RUN=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/run; LOGS=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/logs
BIN=${TOKULY_BIN:-${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/target/main/debug/tokuly}; [ -x "$BIN" ] || BIN=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/bin/tokuly-dev-api
# The addresses the API at port $1 hands browsers, for the web server's Content-Security-Policy.
browser_addresses() {
  eval "$("$REPO/tools/dev/infra.sh" env | grep -E 'TOKULY_(S3_PUBLIC_ENDPOINT|LIVEKIT_PUBLIC_URL|LIVE_(WHIP|WHEP|HLS|PLAYBACK)_URL)=')"
  export TOKULY_S3_PUBLIC_ENDPOINT TOKULY_LIVEKIT_PUBLIC_URL TOKULY_LIVE_WHIP_URL TOKULY_LIVE_WHEP_URL TOKULY_LIVE_HLS_URL TOKULY_LIVE_PLAYBACK_URL
  if [ "$1" = 58081 ]; then
    export TOKULY_RT_PUBLIC_WT_URL=https://127.0.0.1:54433/rt TOKULY_RT_PUBLIC_WS_URL=ws://localhost:58082/rt/ws
  else
    export TOKULY_RT_PUBLIC_WT_URL="https://127.0.0.1:$(($1 + 2))/rt" TOKULY_RT_PUBLIC_WS_URL="ws://localhost:$(($1 + 1))/rt/ws"
    [ -z "${TOKULY_LIVE_WHIP_URL:-}" ] || pair_media_urls "$1"
  fi
}
# The media server of the API at port $1 (`api-here`) on <port>+3 to <port>+9. MediaMTX asks one API
# whether a stream may be published, and an API kicks every publisher it does not know: on the
# shared media server a second API's streams are refused, and it kicks the first one's.
pair_media_urls() {
  export TOKULY_LIVE_HLS_URL="http://127.0.0.1:$(($1 + 3))" TOKULY_LIVE_WHIP_URL="http://127.0.0.1:$(($1 + 4))"
  export TOKULY_LIVE_WHEP_URL="$TOKULY_LIVE_WHIP_URL" TOKULY_MEDIAMTX_API_URL="http://127.0.0.1:$(($1 + 6))"
  export TOKULY_LIVE_PLAYBACK_URL="http://127.0.0.1:$(($1 + 7))" TOKULY_LIVE_RTMP_URL="rtmp://127.0.0.1:$(($1 + 8))"
  export TOKULY_LIVE_SRT_URL="srt://127.0.0.1:$(($1 + 9))"
}
pair_mediamtx() { # name port
  local recordings="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/data/mediamtx-$1/recordings"
  stop_one "mediamtx-$1"; pair_media_urls "$2"; mkdir -p "$recordings"
  export TOKULY_LIVE_RECORDINGS_DIR="$recordings"
  MTX_AUTHHTTPADDRESS="http://mediamtx:$TOKULY_LIVE_HOOK_SECRET@127.0.0.1:$2/hooks/live/auth" \
    MTX_HLSADDRESS="127.0.0.1:$(($2 + 3))" MTX_WEBRTCADDRESS="127.0.0.1:$(($2 + 4))" \
    MTX_WEBRTCLOCALUDPADDRESS="127.0.0.1:$(($2 + 5))" MTX_APIADDRESS="127.0.0.1:$(($2 + 6))" \
    MTX_PLAYBACKADDRESS="127.0.0.1:$(($2 + 7))" MTX_RTMPADDRESS="127.0.0.1:$(($2 + 8))" \
    MTX_SRTADDRESS="127.0.0.1:$(($2 + 9))" MTX_PATHDEFAULTS_RECORDPATH="$recordings/%path/%s-%f" \
    nohup "${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/bin/mediamtx" "$REPO/infra/mediamtx/mediamtx.yml" \
    >"$LOGS/mediamtx-$1.log" 2>&1 & echo $! >"$RUN/mediamtx-$1.pid"
}
# The tokuly.toml a Compose deployment starts with, so the browser tests meet the same settings.
first_config() { "$REPO/tools/init-env.sh" --config "$RUN/tokuly-$1.toml"; export TOKULY_CONFIG="$RUN/tokuly-$1.toml"; }
# A binary built before the checkout's last backend or proto commit tests an API the checkout no longer has.
not_older_than_checkout() {
  local changed; changed=$(git log -1 --format=%ct -- "$(git rev-parse --show-toplevel)"/{backend,proto})
  [ "$(stat -c %Y "$1")" -ge "$changed" ] || {
    echo "$1 is older than this checkout's last backend change ($(git log -1 --format='%h %s' -- "$(git rev-parse --show-toplevel)"/{backend,proto})); build it from this checkout"
    exit 1
  }
}
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
    first_config dev
    export TOKULY_PUBLIC_URL=http://localhost:53000 TOKULY_API_LISTEN=127.0.0.1:58081
    # The account with this handle becomes admin when it signs up (used by the browser tests).
    # Browser tests sign up many accounts from one address.
    export TOKULY_ADMIN_USER=e2e-admin@e2e.tokuly.test TOKULY_RATE_LIMITS=false TOKULY_NEWS_ALLOW_LOOPBACK=true TOKULY_DEV_MINI_APP_ALLOW_LOOPBACK=true
    export TOKULY_RT_WS_LISTEN=127.0.0.1:58082 TOKULY_RT_WT_LISTEN=0.0.0.0:54433
    export TOKULY_RT_PUBLIC_WT_URL=https://127.0.0.1:54433/rt TOKULY_RT_PUBLIC_WS_URL=ws://localhost:58082/rt/ws
    # Web engineers in their own checkouts run dev servers on 53010-53039 against this API.
    export TOKULY_RT_ALLOWED_ORIGINS="$(for p in $(seq 53010 53039); do printf 'http://localhost:%s,' "$p"; done | sed 's/,$//')"
    nohup "$BIN" all >"$LOGS/tokuly.log" 2>&1 & echo $! >"$RUN/api.pid" ;;
  web)
    stop_one web
    cd "$REPO/apps/web"; browser_addresses 58081
    TOKULY_DEV_API_URL=http://127.0.0.1:58081 TOKULY_DEV_TILES_URL=http://127.0.0.1:58090 \
      TOKULY_DEV_MEDIA_URL=http://127.0.0.1:59000/tokuly-media \
      nohup pnpm exec vite dev --port 53000 --host 127.0.0.1 >"$LOGS/web.log" 2>&1 & echo $! >"$RUN/web.pid" ;;
  api-here)
    # A complete, separate backend for the checkout you are in: its own database, cache, event
    # streams, analytics database and search indexes, so it can run next to the shared API.
    #   TOKULY_BIN=<your build>/debug/tokuly dev.sh api-here <name> <api port> <web port>
    # Listens on <api port> (API), <api port>+1 (realtime WebSocket), <api port>+2 (WebTransport);
    # while the shared MediaMTX runs, the pair gets its own on <api port>+3 to +9 (`pair_mediamtx`).
    # The database is recreated on every start. logs: $TOKULY_DEV_HOME/logs/api-<name>.log
    name=${2:?name}; port=${3:?api port}; webport=${4:?web port}
    [ -n "${TOKULY_BIN:-}" ] || { echo "set TOKULY_BIN to the tokuly binary of your build"; exit 1; }
    not_older_than_checkout "$TOKULY_BIN"
    stop_one "api-$name"
    eval "$("$REPO/tools/dev/infra.sh" env)"
    ADMIN_URL="$TOKULY_DATABASE_URL"; db="tokuly_${name//[^a-z0-9_]/_}"
    psql "$ADMIN_URL" -qc "DROP DATABASE IF EXISTS $db WITH (FORCE)" -c "CREATE DATABASE $db" >/dev/null
    curl -fsS "$TOKULY_CLICKHOUSE_URL/" --user "$TOKULY_CLICKHOUSE_USER:$TOKULY_CLICKHOUSE_PASSWORD" \
      --data-binary "CREATE DATABASE IF NOT EXISTS $db" >/dev/null
    export TOKULY_DATABASE_URL="${ADMIN_URL%/*}/$db" TOKULY_CLICKHOUSE_DATABASE="$db"
    first_config "$name"
    export TOKULY_VALKEY_URL="${TOKULY_VALKEY_URL%/}/$(( port % 14 + 2 ))"
    export TOKULY_NATS_PREFIX="${name//[^a-zA-Z0-9_]/_}" TOKULY_SEARCH_INDEX_PREFIX="${name//[^a-zA-Z0-9_]/_}_"
    export TOKULY_PUBLIC_URL="http://localhost:$webport" TOKULY_API_LISTEN="127.0.0.1:$port"
    export TOKULY_ADMIN_USER=e2e-admin@e2e.tokuly.test TOKULY_RATE_LIMITS=false TOKULY_NEWS_ALLOW_LOOPBACK=true TOKULY_DEV_MINI_APP_ALLOW_LOOPBACK=true
    export TOKULY_RT_WS_LISTEN="127.0.0.1:$((port + 1))" TOKULY_RT_WT_LISTEN="0.0.0.0:$((port + 2))"
    export TOKULY_RT_PUBLIC_WT_URL="https://127.0.0.1:$((port + 2))/rt" TOKULY_RT_PUBLIC_WS_URL="ws://localhost:$((port + 1))/rt/ws"
    [ -z "${TOKULY_MEDIAMTX_API_URL:-}" ] || pair_mediamtx "$name" "$port"
    nohup "$TOKULY_BIN" all >"$LOGS/api-$name.log" 2>&1 & echo $! >"$RUN/api-$name.pid" ;;
  api-here-stop) stop_one "api-${2:?name}"; stop_one "mediamtx-$2" ;;
  web-here)
    # A dev server for the checkout you are in (a git worktree):
    #   dev.sh web-here <name> <port> [<api port>]     default API: the shared one on 58081
    # logs: $TOKULY_DEV_HOME/logs/web-<name>.log
    name=${2:?name}; port=${3:?port}; api=${4:-58081}
    stop_one "web-$name"
    cd "$(git rev-parse --show-toplevel)/apps/web"; browser_addresses "$api"
    TOKULY_DEV_API_URL=http://127.0.0.1:$api TOKULY_DEV_TILES_URL=http://127.0.0.1:58090 \
      TOKULY_DEV_MEDIA_URL=http://127.0.0.1:59000/tokuly-media \
      nohup pnpm exec vite dev --port "$port" --strictPort --host 127.0.0.1 >"$LOGS/web-$name.log" 2>&1 & echo $! >"$RUN/web-$name.pid" ;;
  web-here-stop) stop_one "web-${2:?name}" ;;
  stop) stop_one api; stop_one web ;;
  *) echo "usage: dev.sh api [fresh] | web | api-here <name> <api port> <web port> | api-here-stop <name> | web-here <name> <port> [<api port>] | web-here-stop <name> | stop"; exit 1 ;;
esac
