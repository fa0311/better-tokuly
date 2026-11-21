#!/usr/bin/env bash
# Prints the `-p <crate>` flags that CI builds and tests for a change.
#
#   tools/ci/backend-packages.sh            everything the server is built from
#   tools/ci/backend-packages.sh <base>     for the changes since <base> (a pull request):
#                                           the crates it touches, or everything the server is
#                                           built from when the shared foundation changed
#
# "Everything the server is built from" rather than the whole workspace: crates that are
# still being written are not wired into the server yet and may not build.
set -euo pipefail
cd "$(dirname "$0")/../.."

# Workspace crates reachable from the server, by walking their tokuly-* dependencies.
integrated() {
  cargo metadata --manifest-path backend/Cargo.toml --no-deps --format-version 1 | jq -r '
    (.packages | map({key: .name, value: [.dependencies[].name | select(startswith("tokuly-"))]}) | from_entries) as $deps
    | def closure($seen; $queue):
        if ($queue | length) == 0 then $seen
        else ($queue[0]) as $name
          | if ($seen | index($name)) then closure($seen; $queue[1:])
            else closure($seen + [$name]; $queue[1:] + ($deps[$name] // []))
            end
        end;
      closure([]; ["tokuly-server"]) | sort | .[]'
}

base="${1:-}"
if [ -z "$base" ]; then
  crates="$(integrated)"
else
  changed_files="$(git diff --name-only "$base"...HEAD)"
  # Crates whose own directory changed (the directory name is the crate name without `tokuly-`).
  crates="$(printf '%s\n' "$changed_files" | sed -n 's|^backend/crates/\([a-z_-]*\)/.*|tokuly-\1|p' | sort -u)"
  # A change to what every crate builds on has to be checked against all of them.
  if printf '%s\n' "$changed_files" | grep -Eq '^(backend/crates/(core|proto|testkit|server|identity)/|backend/Cargo\.toml$|proto/tokuly/(common|realtime|identity)/)'; then
    crates="$(printf '%s\n%s\n' "$crates" "$(integrated)" | sort -u)"
  fi
fi

flags=""
for crate in $crates; do
  # A crate directory that was deleted by the change has nothing to build.
  [ -d "backend/crates/${crate#tokuly-}" ] && flags="$flags -p $crate"
done
echo "${flags# }"
