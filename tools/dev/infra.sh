#!/usr/bin/env bash
# Runs Tokuly's backing services natively (no Docker) for backend development
# and for machines where a container runtime is unavailable.
#
#   tools/dev/infra.sh install   download the service binaries
#   tools/dev/infra.sh up        start everything in the background
#   tools/dev/infra.sh down      stop everything
#   tools/dev/infra.sh status    show what is running
#   tools/dev/infra.sh env       print `export TOKULY_...` lines for this setup
#
# PostgreSQL (with PostGIS) and Valkey are expected from the OS package
# manager; everything else is downloaded into $TOKULY_DEV_HOME/bin.
# Ports can be overridden by exporting the P_* variables below or by putting
# them in $TOKULY_DEV_HOME/ports.env.
#
# The media server for live streams (MediaMTX) is started on request only:
#   TOKULY_DEV_MEDIAMTX=1 tools/dev/infra.sh up
# because the live domain's own test starts one on the same ports. While it
# runs, `env` also prints the TOKULY_LIVE_* settings that point the API at it.
set -euo pipefail

DEV_HOME="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}"
BIN="$DEV_HOME/bin"
DATA="$DEV_HOME/data"
LOGS="$DEV_HOME/logs"
RUN="$DEV_HOME/run"
mkdir -p "$BIN" "$DATA" "$LOGS" "$RUN"
# shellcheck disable=SC1091
[ -f "$DEV_HOME/ports.env" ] && . "$DEV_HOME/ports.env"

: "${P_POSTGRES:=5432}" "${P_VALKEY:=6379}" "${P_NATS:=4222}" "${P_NATS_HTTP:=8222}"
: "${P_S3:=9000}" "${P_S3_CONSOLE:=9001}" "${P_CLICKHOUSE_HTTP:=8123}" "${P_CLICKHOUSE_TCP:=9440}"
: "${P_MEILI:=7700}" "${P_SMTP:=1025}" "${P_MAILPIT:=8025}"
: "${P_MEDIAMTX_RTMP:=1935}" "${P_MEDIAMTX_HLS:=8888}" "${P_MEDIAMTX_WEBRTC:=8889}" "${P_MEDIAMTX_API:=9997}"
: "${P_MEDIAMTX_SRT:=8890}" "${P_MEDIAMTX_ICE:=8189}" "${P_MEDIAMTX_RTSP:=8554}"
# The rewinding (DVR) server sits one below the control API, as in mediamtx.yml.
: "${P_MEDIAMTX_PLAYBACK:=$((P_MEDIAMTX_API - 1))}"
: "${P_LIVEKIT:=7880}" "${P_LIVEKIT_TCP:=7881}" "${P_LIVEKIT_UDP:=7882}"
# Where the API listens (TOKULY_API_LISTEN): LiveKit posts its webhook there.
: "${P_API:=8081}"
: "${PG_BIN:=$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1)}"

NATS_VERSION=2.15.3
MEILI_VERSION=1.57.0
RUSTFS_VERSION=1.0.6
CLICKHOUSE_VERSION=26.9.33.10
MAILPIT_VERSION=1.34.0
MEDIAMTX_VERSION=1.21.4
LIVEKIT_VERSION=1.13.11
PMTILES_VERSION=1.31.2

S3_ACCESS_KEY="${TOKULY_S3_ACCESS_KEY:-tokuly}"
S3_SECRET_KEY="${TOKULY_S3_SECRET_KEY:-tokuly-secret}"
MEILI_KEY="${TOKULY_MEILI_KEY:-tokuly-meili-key}"
LIVEKIT_KEY="${TOKULY_LIVEKIT_API_KEY:-tokulydev}"
LIVEKIT_SECRET="${TOKULY_LIVEKIT_API_SECRET:-tokuly-livekit-dev-secret-0123456789}"
# Two different secrets: what MediaMTX presents to the API's hook, and what
# the worker presents to MediaMTX's control API.
LIVE_HOOK_SECRET="${TOKULY_LIVE_HOOK_SECRET:-tokuly-live-hook-dev-secret-0123456789}"
MEDIAMTX_API_SECRET="${TOKULY_MEDIAMTX_API_SECRET:-tokuly-mediamtx-api-dev-secret-0123456789}"
RECORDINGS="$DATA/mediamtx/recordings"
REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

fetch() { # url dest
  echo "  downloading $(basename "$2")"
  curl -fsSL --retry 3 -o "$2" "$1"
}

install_bins() {
  local tmp; tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' RETURN
  if [ ! -x "$BIN/nats-server" ]; then
    fetch "https://github.com/nats-io/nats-server/releases/download/v$NATS_VERSION/nats-server-v$NATS_VERSION-linux-amd64.tar.gz" "$tmp/nats.tgz"
    tar -xzf "$tmp/nats.tgz" -C "$tmp" && install -m 0755 "$tmp/nats-server-v$NATS_VERSION-linux-amd64/nats-server" "$BIN/nats-server"
  fi
  if [ ! -x "$BIN/meilisearch" ]; then
    fetch "https://github.com/meilisearch/meilisearch/releases/download/v$MEILI_VERSION/meilisearch-linux-amd64" "$tmp/meilisearch"
    install -m 0755 "$tmp/meilisearch" "$BIN/meilisearch"
  fi
  if [ ! -x "$BIN/rustfs" ]; then
    fetch "https://github.com/rustfs/rustfs/releases/download/$RUSTFS_VERSION/rustfs-linux-x86_64-gnu-v$RUSTFS_VERSION.zip" "$tmp/rustfs.zip"
    unzip -q -o "$tmp/rustfs.zip" -d "$tmp/rustfs" && install -m 0755 "$(find "$tmp/rustfs" -type f -name rustfs | head -1)" "$BIN/rustfs"
  fi
  if [ ! -x "$BIN/clickhouse" ]; then
    fetch "https://github.com/ClickHouse/ClickHouse/releases/download/v$CLICKHOUSE_VERSION-stable/clickhouse-common-static-$CLICKHOUSE_VERSION-amd64.tgz" "$tmp/ch.tgz"
    tar -xzf "$tmp/ch.tgz" -C "$tmp" && install -m 0755 "$(find "$tmp" -type f -path '*usr/bin/clickhouse' | head -1)" "$BIN/clickhouse"
  fi
  if [ ! -x "$BIN/mailpit" ]; then
    fetch "https://github.com/axllent/mailpit/releases/download/v$MAILPIT_VERSION/mailpit-linux-amd64.tar.gz" "$tmp/mailpit.tgz"
    tar -xzf "$tmp/mailpit.tgz" -C "$tmp" && install -m 0755 "$tmp/mailpit" "$BIN/mailpit"
  fi
  if [ ! -x "$BIN/mediamtx" ]; then
    fetch "https://github.com/bluenviron/mediamtx/releases/download/v$MEDIAMTX_VERSION/mediamtx_v${MEDIAMTX_VERSION}_linux_amd64.tar.gz" "$tmp/mediamtx.tgz"
    mkdir -p "$tmp/mediamtx" && tar -xzf "$tmp/mediamtx.tgz" -C "$tmp/mediamtx" && install -m 0755 "$tmp/mediamtx/mediamtx" "$BIN/mediamtx"
  fi
  if [ ! -x "$BIN/livekit-server" ]; then
    fetch "https://github.com/livekit/livekit/releases/download/v$LIVEKIT_VERSION/livekit_${LIVEKIT_VERSION}_linux_amd64.tar.gz" "$tmp/livekit.tgz"
    mkdir -p "$tmp/livekit" && tar -xzf "$tmp/livekit.tgz" -C "$tmp/livekit" && install -m 0755 "$tmp/livekit/livekit-server" "$BIN/livekit-server"
  fi
  if [ ! -x "$BIN/pmtiles" ]; then
    fetch "https://github.com/protomaps/go-pmtiles/releases/download/v$PMTILES_VERSION/go-pmtiles_${PMTILES_VERSION}_Linux_x86_64.tar.gz" "$tmp/pmtiles.tgz"
    mkdir -p "$tmp/pmtiles" && tar -xzf "$tmp/pmtiles.tgz" -C "$tmp/pmtiles" && install -m 0755 "$tmp/pmtiles/pmtiles" "$BIN/pmtiles"
  fi
  echo "binaries ready in $BIN"
}

running() { [ -f "$RUN/$1.pid" ] && kill -0 "$(cat "$RUN/$1.pid")" 2>/dev/null; }

start() { # name command...
  local name="$1"; shift
  if running "$name"; then echo "  $name already running"; return; fi
  nohup "$@" >"$LOGS/$name.log" 2>&1 &
  echo $! >"$RUN/$name.pid"
  echo "  started $name (pid $!)"
}

stop() {
  local name="$1"
  if running "$name"; then kill "$(cat "$RUN/$name.pid")" 2>/dev/null || true; echo "  stopped $name"; fi
  rm -f "$RUN/$name.pid"
}

wait_port() { # name port
  for _ in $(seq 1 100); do
    if (exec 3<>"/dev/tcp/127.0.0.1/$2") 2>/dev/null; then exec 3>&- 3<&-; return 0; fi
    sleep 0.2
  done
  echo "  WARNING: $1 did not open port $2 (see $LOGS/$1.log)"; return 1
}

up_postgres() {
  local dir="$DATA/postgres"
  if [ ! -f "$dir/PG_VERSION" ]; then
    "$PG_BIN/initdb" -D "$dir" -U tokuly --auth=trust --encoding=UTF8 --locale=C.UTF-8 >"$LOGS/initdb.log" 2>&1
    cat >>"$dir/postgresql.conf" <<EOF
port = $P_POSTGRES
listen_addresses = '127.0.0.1'
unix_socket_directories = '/tmp'
max_connections = 300
shared_buffers = 512MB
EOF
  fi
  if ! "$PG_BIN/pg_isready" -q -h 127.0.0.1 -p "$P_POSTGRES"; then
    "$PG_BIN/pg_ctl" -D "$dir" -l "$LOGS/postgres.log" -w start >/dev/null
    echo "  started postgres"
  else
    echo "  postgres already running"
  fi
  psql -h 127.0.0.1 -p "$P_POSTGRES" -U tokuly -d postgres -tAc "select 1 from pg_database where datname='tokuly'" | grep -q 1 \
    || createdb -h 127.0.0.1 -p "$P_POSTGRES" -U tokuly tokuly
}

up() {
  [ -x "$BIN/nats-server" ] || install_bins
  echo "starting services (data: $DATA, logs: $LOGS)"
  up_postgres
  mkdir -p "$DATA/valkey" "$DATA/nats" "$DATA/rustfs" "$DATA/clickhouse" "$DATA/meili" "$DATA/mediamtx"
  start valkey valkey-server --port "$P_VALKEY" --bind 127.0.0.1 --dir "$DATA/valkey" --save 60 100 --appendonly no
  start nats "$BIN/nats-server" -a 127.0.0.1 -p "$P_NATS" -m "$P_NATS_HTTP" -js -sd "$DATA/nats"
  RUSTFS_ACCESS_KEY="$S3_ACCESS_KEY" RUSTFS_SECRET_KEY="$S3_SECRET_KEY" \
    start rustfs "$BIN/rustfs" server --address "127.0.0.1:$P_S3" --console-enable \
    --console-address "127.0.0.1:$P_S3_CONSOLE" "$DATA/rustfs"
  (cd "$DATA/clickhouse" && start clickhouse "$BIN/clickhouse" server -- \
    --path="$DATA/clickhouse/" --http_port="$P_CLICKHOUSE_HTTP" --tcp_port="$P_CLICKHOUSE_TCP" \
    --mysql_port=0 --postgresql_port=0 --interserver_http_port=0 --listen_host=127.0.0.1 \
    --logger.console=1 --logger.level=warning)
  start meili "$BIN/meilisearch" --http-addr "127.0.0.1:$P_MEILI" --master-key "$MEILI_KEY" \
    --db-path "$DATA/meili/data.ms" --dump-dir "$DATA/meili/dumps" --no-analytics --env development
  start mailpit "$BIN/mailpit" --smtp "127.0.0.1:$P_SMTP" --listen "127.0.0.1:$P_MAILPIT" --database "$DATA/mailpit.db"
  if [ "${TOKULY_DEV_MEDIAMTX:-0}" = "1" ] || [ -f "${TOKULY_MEDIAMTX_CONFIG:-}" ]; then
    # The configuration the Compose stack ships, moved onto this machine: its
    # ports, the hook of the API running here, and a local recordings folder.
    mkdir -p "$RECORDINGS"
    MTX_AUTHHTTPADDRESS="http://mediamtx:$LIVE_HOOK_SECRET@127.0.0.1:$P_API/hooks/live/auth" \
      MTX_APIADDRESS="127.0.0.1:$P_MEDIAMTX_API" MTX_PLAYBACKADDRESS="127.0.0.1:$P_MEDIAMTX_PLAYBACK" \
      MTX_RTMPADDRESS="127.0.0.1:$P_MEDIAMTX_RTMP" MTX_SRTADDRESS="127.0.0.1:$P_MEDIAMTX_SRT" \
      MTX_HLSADDRESS="127.0.0.1:$P_MEDIAMTX_HLS" MTX_WEBRTCADDRESS="127.0.0.1:$P_MEDIAMTX_WEBRTC" \
      MTX_WEBRTCLOCALUDPADDRESS="127.0.0.1:$P_MEDIAMTX_ICE" MTX_PATHDEFAULTS_RECORDPATH="$RECORDINGS/%path/%s-%f" \
      start mediamtx "$BIN/mediamtx" "${TOKULY_MEDIAMTX_CONFIG:-$REPO/infra/mediamtx/mediamtx.yml}"
  fi
  if [ "${TOKULY_DEV_LIVEKIT:-1}" = "1" ]; then
    start livekit "$BIN/livekit-server" --config-body "$(printf 'port: %s\nbind_addresses: ["127.0.0.1"]\nrtc:\n  tcp_port: %s\n  udp_port: %s\n  use_external_ip: false\nroom:\n  auto_create: false\nkeys:\n  %s: %s\nwebhook:\n  api_key: %s\n  urls:\n    - http://127.0.0.1:%s/spaces/livekit/webhook\nlogging:\n  level: info\n' \
      "$P_LIVEKIT" "$P_LIVEKIT_TCP" "$P_LIVEKIT_UDP" "$LIVEKIT_KEY" "$LIVEKIT_SECRET" "$LIVEKIT_KEY" "$P_API")"
  fi
  wait_port valkey "$P_VALKEY" || true
  wait_port nats "$P_NATS" || true
  wait_port rustfs "$P_S3" || true
  wait_port clickhouse "$P_CLICKHOUSE_HTTP" || true
  wait_port meili "$P_MEILI" || true
  curl -fsS "http://127.0.0.1:$P_CLICKHOUSE_HTTP/" --data-binary "CREATE DATABASE IF NOT EXISTS tokuly" >/dev/null 2>&1 || true
  echo "infrastructure is up. Load the matching settings with:  eval \"\$(tools/dev/infra.sh env)\""
}

down() {
  for s in livekit mediamtx mailpit meili clickhouse rustfs nats valkey; do stop "$s"; done
  if "$PG_BIN/pg_isready" -q -h 127.0.0.1 -p "$P_POSTGRES"; then
    "$PG_BIN/pg_ctl" -D "$DATA/postgres" -m fast stop >/dev/null && echo "  stopped postgres"
  fi
}

status() {
  "$PG_BIN/pg_isready" -q -h 127.0.0.1 -p "$P_POSTGRES" && echo "postgres    up   :$P_POSTGRES" || echo "postgres    down"
  for s in valkey nats rustfs clickhouse meili mailpit mediamtx livekit; do
    if running "$s"; then printf "%-11s up   pid %s\n" "$s" "$(cat "$RUN/$s.pid")"; else printf "%-11s down\n" "$s"; fi
  done
}

print_env() {
  cat <<EOF
export TOKULY_DATABASE_URL="postgres://tokuly@127.0.0.1:$P_POSTGRES/tokuly"
export TOKULY_VALKEY_URL="redis://127.0.0.1:$P_VALKEY"
export TOKULY_NATS_URL="nats://127.0.0.1:$P_NATS"
export TOKULY_S3_ENDPOINT="http://127.0.0.1:$P_S3"
export TOKULY_S3_PUBLIC_ENDPOINT="http://127.0.0.1:$P_S3"
export TOKULY_S3_ACCESS_KEY="$S3_ACCESS_KEY"
export TOKULY_S3_SECRET_KEY="$S3_SECRET_KEY"
export TOKULY_CLICKHOUSE_URL="http://127.0.0.1:$P_CLICKHOUSE_HTTP"
export TOKULY_CLICKHOUSE_USER="default"
export TOKULY_CLICKHOUSE_PASSWORD=""
export TOKULY_MEILI_URL="http://127.0.0.1:$P_MEILI"
export TOKULY_MEILI_KEY="$MEILI_KEY"
export TOKULY_SMTP_URL="smtp://127.0.0.1:$P_SMTP"
export TOKULY_LIVEKIT_URL="http://127.0.0.1:$P_LIVEKIT"
export TOKULY_LIVEKIT_PUBLIC_URL="ws://127.0.0.1:$P_LIVEKIT"
export TOKULY_LIVEKIT_API_KEY="$LIVEKIT_KEY"
export TOKULY_LIVEKIT_API_SECRET="$LIVEKIT_SECRET"
export TOKULY_SECRET="\${TOKULY_SECRET:-dev-secret-dev-secret-dev-secret-dev-secret}"
EOF
  # Live streams, only while a media server is there to follow (started here
  # with TOKULY_DEV_MEDIAMTX=1, or the `live` profile of compose.dev.yaml).
  if [ "${TOKULY_DEV_MEDIAMTX:-0}" = "1" ] || running mediamtx; then
    cat <<EOF
export TOKULY_MEDIAMTX_API_URL="http://127.0.0.1:$P_MEDIAMTX_API"
export TOKULY_LIVE_HOOK_SECRET="$LIVE_HOOK_SECRET"
export TOKULY_MEDIAMTX_API_SECRET="$MEDIAMTX_API_SECRET"
export TOKULY_LIVE_RTMP_URL="rtmp://127.0.0.1:$P_MEDIAMTX_RTMP"
export TOKULY_LIVE_SRT_URL="srt://127.0.0.1:$P_MEDIAMTX_SRT"
export TOKULY_LIVE_WHIP_URL="http://127.0.0.1:$P_MEDIAMTX_WEBRTC"
export TOKULY_LIVE_WHEP_URL="http://127.0.0.1:$P_MEDIAMTX_WEBRTC"
export TOKULY_LIVE_HLS_URL="http://127.0.0.1:$P_MEDIAMTX_HLS"
export TOKULY_LIVE_PLAYBACK_URL="http://127.0.0.1:$P_MEDIAMTX_PLAYBACK"
export TOKULY_LIVE_RECORDINGS_DIR="$RECORDINGS"
EOF
  fi
}

case "${1:-}" in
  install) install_bins ;;
  up) up ;;
  down) down ;;
  status) status ;;
  env) print_env ;;
  *) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
