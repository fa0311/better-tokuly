#!/usr/bin/env bash
# Starts the Compose stack and waits until it is fully up: the one-shot migration has
# finished cleanly and every other service is running and, where it has a health check,
# healthy. Prints the service states and fails if anything exits or the wait times out.
#
#   tools/ci/stack-up.sh [<timeout seconds>]     default 900
set -euo pipefail
cd "$(dirname "$0")/../.."
timeout="${1:-900}"

docker compose up -d
deadline=$((SECONDS + timeout))
while :; do
  states="$(docker compose ps -a --format '{{.Service}} {{.State}} {{.Health}} {{.ExitCode}}')"
  # Anything that exited is a failure, except the migration finishing cleanly.
  if grep -Ev '^migrate exited  ?0$' <<<"$states" | grep -q ' exited '; then
    echo "$states"; echo "stack-up: a service exited" >&2; exit 1
  fi
  if grep -q '^migrate exited' <<<"$states" && ! grep -Eq ' (starting|unhealthy) ' <<<"$states"; then
    echo "$states"; exit 0
  fi
  if [ "$SECONDS" -ge "$deadline" ]; then
    echo "$states"; echo "stack-up: not healthy after ${timeout}s" >&2; exit 1
  fi
  sleep 5
done
