#!/usr/bin/env bash
# Starts the Compose stack and waits until it is fully up: the one-shot migration has
# finished cleanly and every other service is running and, where it has a health check,
# healthy. Prints the service states and fails if anything exits or the wait times out;
# on failure it names the services that are not up and prints their last log lines.
#
#   tools/ci/stack-up.sh [<timeout seconds>]     default 900
set -euo pipefail
cd "$(dirname "$0")/../.."
timeout="${1:-900}"

# The services that are not up yet: the migration until it has exited cleanly, every
# other service until it is running and not starting or unhealthy.
not_up() {
  awk '$1 == "migrate" { if (!($2 == "exited" && $NF == "0")) print $1; next }
       $2 != "running" || $3 == "starting" || $3 == "unhealthy" { print $1 }' <<<"$1"
}

fail() {
  echo "$states"
  for service in $(not_up "$states"); do
    echo "::group::$service (last 50 log lines)"
    docker compose logs --no-color --tail 50 "$service" || true
    echo "::endgroup::"
  done
  echo "stack-up: $1: $(not_up "$states" | xargs)" >&2
  exit 1
}

docker compose up -d
deadline=$((SECONDS + timeout))
while :; do
  states="$(docker compose ps -a --format '{{.Service}} {{.State}} {{.Health}} {{.ExitCode}}')"
  # Anything that exited is a failure, except the migration finishing cleanly.
  if grep -Ev '^migrate exited  ?0$' <<<"$states" | grep -q ' exited '; then
    fail "a service exited"
  fi
  if [ -z "$(not_up "$states")" ]; then
    echo "$states"; exit 0
  fi
  if [ "$SECONDS" -ge "$deadline" ]; then
    fail "not healthy after ${timeout}s"
  fi
  sleep 5
done
