#!/usr/bin/env bash
# Checks that tools/backup.sh and tools/restore.sh bring a deployment back: makes
# an account, a note with an image, backs up, deletes the data volumes and .env
# (a new machine), restores, and finds all of it again. Destroys the data of the
# stack it runs against, so it asks for that on the command line.
#
#   tools/ci/backup-roundtrip.sh --destroy-this-stack [<origin>]    default http://localhost
set -euo pipefail
cd "$(dirname "$0")/../.."
[ "${1:-}" = --destroy-this-stack ] || { echo "usage: tools/ci/backup-roundtrip.sh --destroy-this-stack [<origin>]" >&2; exit 2; }
ORIGIN="${2:-http://localhost}"
# Kept when something fails: the backup is in it.
WORK="$(mktemp -d)"; trap '[ $? -ne 0 ] || rm -rf "$WORK"' EXIT

step() { printf '\n== %s\n' "$*"; }
fail() { printf "FAILED: %s\n(the backup is kept in %s)\n" "$*" "$WORK" >&2; exit 1; }

# rpc <Service/Method> <json body> [<access token>] → response JSON on stdout
rpc() {
  local procedure="$1" body="$2" token="${3:-}" auth=()
  [ -n "$token" ] && auth=(-H "authorization: Bearer $token")
  local status
  status="$(curl -sS -o "$WORK/response.json" -w '%{http_code}' -X POST "$ORIGIN/api/$procedure" -H 'content-type: application/json' "${auth[@]}" -d "$body")"
  [ "$status" = 200 ] || fail "$procedure answered HTTP $status: $(head -c 400 "$WORK/response.json")"
  cat "$WORK/response.json"
}

step "An account and a note with an image"
handle="backup_$(date +%s)_$RANDOM"
password="correct horse battery"
token="$(rpc tokuly.identity.v1.AuthService/SignUp "{\"email\":\"$handle@smoke.tokuly.test\",\"password\":\"$password\",\"handle\":\"$handle\",\"displayName\":\"Backup\",\"birthdate\":\"2000-04-02\",\"acceptTerms\":true}" | jq -r .accessToken)"
version="$(rpc tokuly.notes.v1.NoteService/GetNoteRules '{}' "$token" | jq -r .version)"
rpc tokuly.notes.v1.NoteService/AgreeToNoteRules "{\"version\":$version}" "$token" >/dev/null
python3 - "$WORK/image.png" <<'PY'
import struct, sys, zlib
def chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
rows = b"".join(b"\x00" + b"".join(bytes((x * 4, 120, y * 4)) for x in range(64)) for y in range(64))
png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 64, 64, 8, 2, 0, 0, 0)) + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b"")
open(sys.argv[1], "wb").write(png)
PY
upload="$(rpc tokuly.media.v1.MediaService/CreateUpload "{\"purpose\":\"ASSET_PURPOSE_NOTE_IMAGE\",\"fileName\":\"image.png\",\"contentType\":\"image/png\",\"sizeBytes\":\"$(stat -c %s "$WORK/image.png")\"}" "$token")"
headers=(); while IFS=$'\t' read -r name value; do headers+=(-H "$name: $value"); done < <(jq -r '.putHeaders // {} | to_entries[] | "\(.key)\t\(.value)"' <<<"$upload")
curl -fsS -X PUT "${headers[@]}" --data-binary "@$WORK/image.png" "$(jq -r .putUrl <<<"$upload")" >/dev/null || fail "the upload was refused"
rpc tokuly.media.v1.MediaService/CompleteUpload "{\"uploadId\":\"$(jq -r .uploadId <<<"$upload")\"}" "$token" >/dev/null
asset_id="$(jq -r .assetId <<<"$upload")"
for _ in $(seq 1 60); do
  status="$(rpc tokuly.media.v1.MediaService/GetAsset "{\"assetId\":\"$asset_id\"}" "$token" | jq -r .asset.status)"
  [ "$status" = ASSET_STATUS_READY ] && break
  sleep 2
done
[ "$status" = ASSET_STATUS_READY ] || fail "the image was not processed (status $status)"
text="Backed up and restored $handle"
note_id="$(rpc tokuly.notes.v1.NoteService/CreateNote "{\"text\":\"$text\",\"assetIds\":[\"$asset_id\"],\"visibility\":\"VISIBILITY_PUBLIC\"}" "$token" | jq -r .note.id)"
image_url="$(rpc tokuly.notes.v1.NoteService/GetNote "{\"noteId\":\"$note_id\"}" "$token" | jq -r '.note.attachments[0].image.url // .note.attachments[0].url')"
curl -fsS -o "$WORK/before" "$image_url" || fail "the note's image could not be fetched: $image_url"
echo "@$handle, note $note_id, image $image_url"

step "Back up, then lose every data volume and .env"
tools/backup.sh --dir "$WORK/backups"
backup="$(ls -1d "$WORK"/backups/*/)"
project="${COMPOSE_PROJECT_NAME:-$(docker compose config | sed -n 's/^name: //p')}"
docker compose down
for volume in postgres-data storage-data nats-data valkey-data edge-data clickhouse-data search-data; do
  docker volume rm "${project}_$volume" >/dev/null 2>&1 || true
done
rm .env

step "Restore into the empty stack"
tools/restore.sh "$backup"
tools/ci/stack-up.sh

step "Everything is back"
token="$(rpc tokuly.identity.v1.AuthService/SignIn "{\"identifier\":\"$handle\",\"password\":\"$password\"}" | jq -r .accessToken)"
[ -n "$token" ] && [ "$token" != null ] || fail "the account cannot sign in"
note="$(rpc tokuly.notes.v1.NoteService/GetNote "{\"noteId\":\"$note_id\"}" "$token")"
[ "$(jq -r .note.text <<<"$note")" = "$text" ] || fail "the note is not the one written: $note"
curl -fsS -o "$WORK/after" "$image_url" || fail "the note's image is gone: $image_url"
cmp -s "$WORK/before" "$WORK/after" || fail "the note's image changed"
echo "signs in, note and image are back"
for _ in $(seq 1 60); do
  hits="$(rpc tokuly.search.v1.SearchService/Search "{\"query\":\"$handle\",\"kind\":\"SEARCH_KIND_USER\"}" "$token" | jq '.hits | length')"
  [ "$hits" -gt 0 ] && break
  sleep 2
done
[ "$hits" -gt 0 ] || fail "the search index was not rebuilt: @$handle is not found"
echo "search finds @$handle again"

printf '\nThe backup came back.\n'
