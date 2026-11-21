#!/usr/bin/env bash
# End-to-end check of a running stack (`docker compose up`): the things that only work
# when every service is wired to the others correctly.
#
#   tools/ci/smoke.sh [<origin>]        default http://localhost
#
# Environment: MAILPIT_URL (default http://localhost:8025) for the mail check.
# Exits non-zero at the first failed check, naming it.
set -euo pipefail

ORIGIN="${1:-http://localhost}"
MAILPIT_URL="${MAILPIT_URL:-http://localhost:8025}"
WORK="$(mktemp -d)"; trap 'rm -rf "$WORK"' EXIT

step() { printf '\n== %s\n' "$*"; }
fail() { printf 'FAILED: %s\n' "$*" >&2; exit 1; }

# rpc <Service/Method> <json body> [<access token>] → response JSON on stdout
rpc() {
  local procedure="$1" body="$2" token="${3:-}" auth=()
  [ -n "$token" ] && auth=(-H "authorization: Bearer $token")
  local status
  status="$(curl -sS -o "$WORK/response.json" -w '%{http_code}' -X POST "$ORIGIN/api/$procedure" -H 'content-type: application/json' "${auth[@]}" -d "$body")"
  [ "$status" = 200 ] || fail "$procedure answered HTTP $status: $(head -c 400 "$WORK/response.json")"
  cat "$WORK/response.json"
}

step "The web app is served through the edge"
page="$(curl -fsS "$ORIGIN/")" || fail "the landing page did not load"
grep -q '<html' <<<"$page" || fail "the landing page is not HTML"
# Every script and stylesheet the page links must exist (a build whose server and client
# halves disagree about file names renders a page without styles).
assets="$(grep -oE '(src|href)="/[^"]+\.(js|css)"' <<<"$page" | cut -d'"' -f2 | sort -u)"
[ -n "$assets" ] || fail "the landing page links no scripts or stylesheets"
for asset in $assets; do
  curl -fsS -o /dev/null "$ORIGIN$asset" || fail "$asset, linked by the landing page, did not load"
done

step "Sign-up, sign-in and an authenticated call"
handle="smoke_$(date +%s)_$RANDOM"
signup="$(rpc tokuly.identity.v1.AuthService/SignUp "{\"email\":\"$handle@smoke.tokuly.test\",\"password\":\"correct horse battery\",\"handle\":\"$handle\",\"displayName\":\"Smoke\",\"birthdate\":\"2000-04-02\",\"acceptTerms\":true}")"
token="$(jq -r .accessToken <<<"$signup")"
[ -n "$token" ] && [ "$token" != null ] || fail "sign-up returned no access token"
me="$(rpc tokuly.identity.v1.AccountService/GetMe '{}' "$token")"
[ "$(jq -r .me.user.handle <<<"$me")" = "$handle" ] || fail "GetMe did not return the new account"
rpc tokuly.identity.v1.AuthService/SignIn "{\"identifier\":\"$handle\",\"password\":\"correct horse battery\"}" >/dev/null

step "The verification mail arrived"
for _ in $(seq 1 30); do
  if curl -fsS "$MAILPIT_URL/api/v1/search?query=to:$handle@smoke.tokuly.test" | jq -e '.messages | length > 0' >/dev/null; then mail=ok; break; fi
  sleep 2
done
[ "${mail:-}" = ok ] || fail "no mail for the new account reached Mailpit"

step "Upload an image the way a browser does, and fetch it back through /media"
# A 64x64 PNG generated here, so the check needs no fixture file.
python3 - "$WORK/avatar.png" <<'PY'
import struct, sys, zlib
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
rows = b"".join(b"\x00" + b"".join(bytes((x * 4, y * 4, 200)) for x in range(64)) for y in range(64))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
size="$(stat -c %s "$WORK/avatar.png")"
upload="$(rpc tokuly.media.v1.MediaService/CreateUpload "{\"purpose\":\"ASSET_PURPOSE_AVATAR\",\"fileName\":\"avatar.png\",\"contentType\":\"image/png\",\"sizeBytes\":\"$size\"}" "$token")"
put_url="$(jq -r .putUrl <<<"$upload")"; upload_id="$(jq -r .uploadId <<<"$upload")"; asset_id="$(jq -r .assetId <<<"$upload")"
[ -n "$put_url" ] && [ "$put_url" != null ] || fail "CreateUpload returned no upload URL"
headers=(); while IFS=$'\t' read -r name value; do headers+=(-H "$name: $value"); done < <(jq -r '.putHeaders // {} | to_entries[] | "\(.key)\t\(.value)"' <<<"$upload")
curl -fsS -X PUT "${headers[@]}" --data-binary "@$WORK/avatar.png" "$put_url" >/dev/null || fail "the upload to object storage was refused ($put_url)"
rpc tokuly.media.v1.MediaService/CompleteUpload "{\"uploadId\":\"$upload_id\"}" "$token" >/dev/null
for _ in $(seq 1 60); do
  asset="$(rpc tokuly.media.v1.MediaService/GetAsset "{\"assetId\":\"$asset_id\"}" "$token")"
  status="$(jq -r .asset.status <<<"$asset")"
  [ "$status" = ASSET_STATUS_READY ] && break
  [ "$status" = ASSET_STATUS_FAILED ] && fail "the worker could not process the upload"
  sleep 2
done
[ "$status" = ASSET_STATUS_READY ] || fail "the upload was not processed within two minutes (status $status): is the worker running?"
image_url="$(jq -r '.asset.image.url // .asset.url' <<<"$asset")"
case "$image_url" in "$ORIGIN"/media/*) ;; *) fail "the processed image is not served under $ORIGIN/media: $image_url" ;; esac
curl -fsS -o "$WORK/processed" "$image_url" || fail "the processed image could not be fetched: $image_url"
[ -s "$WORK/processed" ] || fail "the processed image is empty"

step "Map tiles, fonts and icons"
curl -fsS -o "$WORK/tile" "$ORIGIN/tiles/world/0/0/0.mvt" && [ -s "$WORK/tile" ] || fail "the world tile 0/0/0 is missing"
curl -fsS -o /dev/null "$ORIGIN/map-assets/fonts/Noto%20Sans%20Regular/0-255.pbf" || fail "map fonts are not served"
curl -fsS -o /dev/null "$ORIGIN/map-assets/sprites/v4/light.json" || fail "map icons are not served"

step "Realtime endpoints"
endpoints="$(rpc tokuly.realtime.v1.RealtimeService/GetEndpoints '{}' "$token")"
jq -e '(.webtransportUrl // "") | startswith("https://")' <<<"$endpoints" >/dev/null || fail "no WebTransport URL: $endpoints"
jq -e '(.webtransportCertHashes // []) | length > 0' <<<"$endpoints" >/dev/null || fail "no certificate hashes for WebTransport: $endpoints"
# The WebSocket fallback, at the address clients are told to use. curl cannot speak
# WebSocket: it holds the upgraded connection until its time limit, which is expected.
websocket_url="$(jq -r .websocketUrl <<<"$endpoints")"
upgrade="$(curl -sS -o /dev/null -w '%{http_code}' --max-time 5 -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: c21va2UtdGVzdC1rZXktMDE=' -H "Origin: $ORIGIN" "${websocket_url/#ws/http}" 2>/dev/null || true)"
[ "$upgrade" = 101 ] || fail "the WebSocket upgrade at $websocket_url answered HTTP $upgrade"

printf '\nAll checks passed.\n'
