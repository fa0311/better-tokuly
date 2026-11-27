#!/usr/bin/env bash
# A build cache kept as one artifact in a container registry instead of the Actions cache
# (which is limited to 10 GB per repository). One tag per kind of cache, overwritten by
# each save; the artifact carries the paths as a zstd tarball and the key it was saved
# under, so a restore can tell an exact hit from an older cache to start from.
#
#   cache.sh restore <reference> <key> <path>...   prints hit=true|false (for $GITHUB_OUTPUT)
#   cache.sh save    <reference> <key> <path>...
#
# Paths are made absolute; a missing artifact or a failed transfer is a miss, never an error.
set -euo pipefail
mode="$1" ref="$2" key="$3"; shift 3
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT

paths=()
for path in "$@"; do
  case "$path" in \~/*) path="$HOME/${path#\~/}" ;; /*) ;; *) path="$PWD/$path" ;; esac
  paths+=("$path")
done

case "$mode" in
  restore)
    if ! oras pull --no-tty -o "$dir" "$ref" >&2; then
      echo "ghcr-cache: nothing at $ref" >&2; echo "hit=false"; exit 0
    fi
    zstd -dc "$dir/cache.tar.zst" | tar -xPf -
    stored="$(cat "$dir/key")"
    echo "ghcr-cache: restored $ref (saved for $stored)" >&2
    [ "$stored" = "$key" ] && echo "hit=true" || echo "hit=false"
    ;;
  save)
    existing=()
    for path in "${paths[@]}"; do [ -e "$path" ] && existing+=("$path"); done
    [ ${#existing[@]} -gt 0 ] || { echo "ghcr-cache: nothing to save" >&2; exit 0; }
    tar -cPf - "${existing[@]}" | zstd -T0 -3 -q -o "$dir/cache.tar.zst"
    printf '%s' "$key" >"$dir/key"
    echo "ghcr-cache: saving $(du -h "$dir/cache.tar.zst" | cut -f1) to $ref" >&2
    (cd "$dir" && oras push --no-tty --artifact-type application/vnd.tokuly.cache "$ref" cache.tar.zst key >&2) ||
      echo "::warning::ghcr-cache: could not save $ref"
    ;;
  *) echo "usage: cache.sh restore|save <reference> <key> <path>..." >&2; exit 2 ;;
esac
