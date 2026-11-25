#!/usr/bin/env bash
# Makes the given images the ones the Compose stack runs, under the names
# compose.yaml derives from TOKULY_IMAGES and TOKULY_VERSION:
#
#   TOKULY_IMAGES=local/tokuly TOKULY_VERSION=ci \
#     tools/ci/use-images.sh backend=ghcr.io/…/ci-backend:sha-… web=ghcr.io/…/web:edge …
#
# Each image is pulled unless it is already here (loaded from an artifact, for
# pull requests from forks), then tagged. Run the stack with
# TOKULY_PULL_POLICY=never so that nothing else is fetched.
set -euo pipefail
: "${TOKULY_IMAGES:?}" "${TOKULY_VERSION:?}"

for pair in "$@"; do
  name="${pair%%=*}" source="${pair#*=}"
  if ! docker image inspect "$source" >/dev/null 2>&1; then
    # One retry: a pull from the registry now and then fails for no reason of ours.
    docker pull -q "$source" || { sleep 10; docker pull -q "$source"; }
  fi
  docker tag "$source" "$TOKULY_IMAGES/$name:$TOKULY_VERSION"
  echo "$name: $source"
done
