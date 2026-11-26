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
# Linux (x86-64) and macOS (Apple silicon). Everything is downloaded into
# $TOKULY_DEV_HOME/bin, at the versions below, except what has no build of its
# own: PostgreSQL (with PostGIS) and Valkey come from the OS package manager
# on Linux and from Homebrew on macOS (`install` runs `brew install`), and so
# does LiveKit on macOS.
# Ports can be overridden by exporting the P_* variables below or by putting
# them in $TOKULY_DEV_HOME/ports.env.
#
# The media server for live streams (MediaMTX) is started on request only:
#   TOKULY_DEV_MEDIAMTX=1 tools/dev/infra.sh up
# because the live domain's own test starts one on the same ports. While it
# runs, `env` also prints the TOKULY_LIVE_* settings that point the API at it.
# LiveKit (audio rooms and calls) starts unless TOKULY_DEV_LIVEKIT=0.
#
# The edge (Caddy with infra/caddy/Caddyfile, the one address the apps talk to:
# /api, /rt/ws, /media, ...) also starts on request only, on port $P_EDGE:
#   TOKULY_DEV_EDGE=1 tools/dev/infra.sh up
# It forwards to the API and the realtime gateway that `tokuly all` runs here,
# to the web app on $P_WEB and the tiles on $P_TILES when they run.
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
# The edge and what it forwards to besides the API.
: "${P_EDGE:=8080}" "${P_RT_WS:=8082}" "${P_WEB:=3000}" "${P_TILES:=8090}"

# The release builds this machine runs.
case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) PLATFORM=linux-amd64 ;;
  Darwin-arm64) PLATFORM=darwin-arm64 ;;
  *) PLATFORM=unsupported ;;
esac
# PostgreSQL 18, as in the Compose stack (the PostGIS of Homebrew is built for it).
if [ "$PLATFORM" = darwin-arm64 ]; then
  BREW_PREFIX="$(brew --prefix 2>/dev/null || echo /opt/homebrew)"
  : "${PG_BIN:=$BREW_PREFIX/opt/postgresql@18/bin}"
  PG_LOCALE=en_US.UTF-8
  # macOS's System V semaphores are few; the backend's pools need far fewer.
  PG_MAX_CONNECTIONS=100
else
  : "${PG_BIN:=$(ls -d /usr/lib/postgresql/*/bin 2>/dev/null | sort -V | tail -1)}"
  PG_LOCALE=C.UTF-8
  PG_MAX_CONNECTIONS=300
fi

NATS_VERSION=2.15.3
MEILI_VERSION=1.57.0
RUSTFS_VERSION=1.0.6
CLICKHOUSE_VERSION=26.9.33.10
MAILPIT_VERSION=1.34.0
MEDIAMTX_VERSION=1.21.4
LIVEKIT_VERSION=1.13.11
PMTILES_VERSION=1.31.2
CADDY_VERSION=2.11.9 # the edge image of compose.yaml

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

# Where each service's build for this machine is released.
release_url() { # service
  case "$1:$PLATFORM" in
    nats-server:linux-amd64) echo "https://github.com/nats-io/nats-server/releases/download/v$NATS_VERSION/nats-server-v$NATS_VERSION-linux-amd64.tar.gz" ;;
    nats-server:darwin-arm64) echo "https://github.com/nats-io/nats-server/releases/download/v$NATS_VERSION/nats-server-v$NATS_VERSION-darwin-arm64.tar.gz" ;;
    meilisearch:linux-amd64) echo "https://github.com/meilisearch/meilisearch/releases/download/v$MEILI_VERSION/meilisearch-linux-amd64" ;;
    meilisearch:darwin-arm64) echo "https://github.com/meilisearch/meilisearch/releases/download/v$MEILI_VERSION/meilisearch-macos-apple-silicon" ;;
    rustfs:linux-amd64) echo "https://github.com/rustfs/rustfs/releases/download/$RUSTFS_VERSION/rustfs-linux-x86_64-gnu-v$RUSTFS_VERSION.zip" ;;
    rustfs:darwin-arm64) echo "https://github.com/rustfs/rustfs/releases/download/$RUSTFS_VERSION/rustfs-macos-aarch64-v$RUSTFS_VERSION.zip" ;;
    clickhouse:linux-amd64) echo "https://github.com/ClickHouse/ClickHouse/releases/download/v$CLICKHOUSE_VERSION-stable/clickhouse-common-static-$CLICKHOUSE_VERSION-amd64.tgz" ;;
    clickhouse:darwin-arm64) echo "https://github.com/ClickHouse/ClickHouse/releases/download/v$CLICKHOUSE_VERSION-stable/clickhouse-macos-aarch64" ;;
    mailpit:linux-amd64) echo "https://github.com/axllent/mailpit/releases/download/v$MAILPIT_VERSION/mailpit-linux-amd64.tar.gz" ;;
    mailpit:darwin-arm64) echo "https://github.com/axllent/mailpit/releases/download/v$MAILPIT_VERSION/mailpit-darwin-arm64.tar.gz" ;;
    mediamtx:linux-amd64) echo "https://github.com/bluenviron/mediamtx/releases/download/v$MEDIAMTX_VERSION/mediamtx_v${MEDIAMTX_VERSION}_linux_amd64.tar.gz" ;;
    mediamtx:darwin-arm64) echo "https://github.com/bluenviron/mediamtx/releases/download/v$MEDIAMTX_VERSION/mediamtx_v${MEDIAMTX_VERSION}_darwin_arm64.tar.gz" ;;
    livekit-server:linux-amd64) echo "https://github.com/livekit/livekit/releases/download/v$LIVEKIT_VERSION/livekit_${LIVEKIT_VERSION}_linux_amd64.tar.gz" ;;
    pmtiles:linux-amd64) echo "https://github.com/protomaps/go-pmtiles/releases/download/v$PMTILES_VERSION/go-pmtiles_${PMTILES_VERSION}_Linux_x86_64.tar.gz" ;;
    pmtiles:darwin-arm64) echo "https://github.com/protomaps/go-pmtiles/releases/download/v$PMTILES_VERSION/go-pmtiles-${PMTILES_VERSION}_Darwin_arm64.zip" ;;
    caddy:linux-amd64) echo "https://github.com/caddyserver/caddy/releases/download/v$CADDY_VERSION/caddy_${CADDY_VERSION}_linux_amd64.tar.gz" ;;
    caddy:darwin-arm64) echo "https://github.com/caddyserver/caddy/releases/download/v$CADDY_VERSION/caddy_${CADDY_VERSION}_mac_arm64.tar.gz" ;;
    *) echo "infra.sh: no release of $1 for $PLATFORM" >&2; return 1 ;;
  esac
}

# Downloads a service into $BIN, unless it is there: an archive (the binary is
# the file at <path> inside it) or the binary itself.
install_bin() { # name [path inside the archive, default: the name]
  local name="$1" member="${2:-$1}" url dir file
  [ -x "$BIN/$name" ] && return 0
  url="$(release_url "$name")"
  dir="$tmp/$name"; mkdir -p "$dir"
  file="$dir/$(basename "$url")"
  fetch "$url" "$file"
  case "$file" in
    *.tar.gz | *.tgz) tar -xzf "$file" -C "$dir" ;;
    *.zip) unzip -q -o "$file" -d "$dir" ;;
    *) mv "$file" "$dir/$member" ;;
  esac
  install -m 0755 "$(find "$dir" -type f -path "*/$member" | head -1)" "$BIN/$name"
}

# What has no build of its own for macOS, from Homebrew (LiveKit there is the
# version pinned above: checked, not forced).
brew_install() {
  local wanted=(postgresql@18 postgis valkey) missing=() formula
  [ "${TOKULY_DEV_LIVEKIT:-1}" = "1" ] && wanted+=(livekit)
  for formula in "${wanted[@]}"; do brew list --formula "$formula" >/dev/null 2>&1 || missing+=("$formula"); done
  if [ "${#missing[@]}" -gt 0 ]; then
    echo "  brew install ${missing[*]}"
    HOMEBREW_NO_AUTO_UPDATE=1 HOMEBREW_NO_INSTALL_CLEANUP=1 brew install --quiet "${missing[@]}"
  fi
  if [ "${TOKULY_DEV_LIVEKIT:-1}" = "1" ]; then
    ln -sf "$BREW_PREFIX/bin/livekit-server" "$BIN/livekit-server"
    "$BIN/livekit-server" --version | grep -q "$LIVEKIT_VERSION" \
      || echo "  WARNING: Homebrew's LiveKit is not $LIVEKIT_VERSION: $("$BIN/livekit-server" --version)"
  fi
}

install_bins() {
  if [ "$PLATFORM" = unsupported ]; then
    echo "infra.sh: runs on Linux x86-64 and macOS on Apple silicon, not $(uname -s) $(uname -m)" >&2; exit 1
  fi
  local tmp; tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"; trap - RETURN' RETURN
  [ "$PLATFORM" = darwin-arm64 ] && brew_install
  install_bin nats-server
  install_bin meilisearch
  install_bin rustfs
  if [ "$PLATFORM" = darwin-arm64 ]; then install_bin clickhouse; else install_bin clickhouse usr/bin/clickhouse; fi
  install_bin mailpit
  install_bin mediamtx
  [ "${TOKULY_DEV_LIVEKIT:-1}" = "1" ] && install_bin livekit-server
  install_bin pmtiles
  [ "${TOKULY_DEV_EDGE:-0}" = "1" ] && install_bin caddy
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
    "$PG_BIN/initdb" -D "$dir" -U tokuly --auth=trust --encoding=UTF8 --locale="$PG_LOCALE" >"$LOGS/initdb.log" 2>&1
    cat >>"$dir/postgresql.conf" <<EOF
port = $P_POSTGRES
listen_addresses = '127.0.0.1'
unix_socket_directories = '/tmp'
max_connections = $PG_MAX_CONNECTIONS
shared_buffers = 512MB
EOF
  fi
  if ! "$PG_BIN/pg_isready" -q -h 127.0.0.1 -p "$P_POSTGRES"; then
    "$PG_BIN/pg_ctl" -D "$dir" -l "$LOGS/postgres.log" -w start >/dev/null
    echo "  started postgres"
  else
    echo "  postgres already running"
  fi
  "$PG_BIN/psql" -h 127.0.0.1 -p "$P_POSTGRES" -U tokuly -d postgres -tAc "select 1 from pg_database where datname='tokuly'" | grep -q 1 \
    || "$PG_BIN/createdb" -h 127.0.0.1 -p "$P_POSTGRES" -U tokuly tokuly
}

up() {
  install_bins
  echo "starting services (data: $DATA, logs: $LOGS)"
  up_postgres
  mkdir -p "$DATA/valkey" "$DATA/nats" "$DATA/rustfs" "$DATA/clickhouse" "$DATA/meili" "$DATA/mediamtx"
  start valkey valkey-server --port "$P_VALKEY" --bind 127.0.0.1 --dir "$DATA/valkey" --save 60 100 --appendonly no
  start nats "$BIN/nats-server" -a 127.0.0.1 -p "$P_NATS" -m "$P_NATS_HTTP" -js -sd "$DATA/nats"
  RUSTFS_ACCESS_KEY="$S3_ACCESS_KEY" RUSTFS_SECRET_KEY="$S3_SECRET_KEY" \
    start rustfs "$BIN/rustfs" server --address "127.0.0.1:$P_S3" --console-enable \
    --console-address "127.0.0.1:$P_S3_CONSOLE" "$DATA/rustfs"
  # The `default` user without a password, written down: the macOS build has
  # no such user of its own when it runs without a configuration.
  cat >"$DATA/clickhouse/users.xml" <<'EOF'
<clickhouse>
  <profiles><default/></profiles>
  <quotas><default/></quotas>
  <users>
    <default>
      <password></password>
      <networks><ip>127.0.0.1</ip><ip>::1</ip></networks>
      <profile>default</profile>
      <quota>default</quota>
      <access_management>1</access_management>
    </default>
  </users>
</clickhouse>
EOF
  (cd "$DATA/clickhouse" && start clickhouse "$BIN/clickhouse" server -- \
    --path="$DATA/clickhouse/" --users_config="$DATA/clickhouse/users.xml" \
    --http_port="$P_CLICKHOUSE_HTTP" --tcp_port="$P_CLICKHOUSE_TCP" \
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
  if [ "${TOKULY_DEV_EDGE:-0}" = "1" ]; then
    # The Compose stack's edge, its services moved onto this machine.
    mkdir -p "$DATA/edge"
    XDG_DATA_HOME="$DATA/edge" XDG_CONFIG_HOME="$DATA/edge" TOKULY_SITE_ADDRESS=":$P_EDGE" \
      TOKULY_EDGE_API="127.0.0.1:$P_API" TOKULY_EDGE_REALTIME="127.0.0.1:$P_RT_WS" \
      TOKULY_EDGE_STORAGE="127.0.0.1:$P_S3" TOKULY_EDGE_TILES="127.0.0.1:$P_TILES" \
      TOKULY_EDGE_WEB="127.0.0.1:$P_WEB" TOKULY_EDGE_MAP_ASSETS="$DATA/map-assets" \
      start edge "$BIN/caddy" run --config "$REPO/infra/caddy/Caddyfile" --adapter caddyfile
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
  for s in edge livekit mediamtx mailpit meili clickhouse rustfs nats valkey; do stop "$s"; done
  if "$PG_BIN/pg_isready" -q -h 127.0.0.1 -p "$P_POSTGRES"; then
    "$PG_BIN/pg_ctl" -D "$DATA/postgres" -m fast stop >/dev/null && echo "  stopped postgres"
  fi
}

status() {
  "$PG_BIN/pg_isready" -q -h 127.0.0.1 -p "$P_POSTGRES" && echo "postgres    up   :$P_POSTGRES" || echo "postgres    down"
  for s in valkey nats rustfs clickhouse meili mailpit mediamtx livekit edge; do
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
