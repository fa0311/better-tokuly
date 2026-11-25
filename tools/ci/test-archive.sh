#!/usr/bin/env bash
# The backend's test binaries, built once and run on several machines.
#
#   tools/ci/test-archive.sh pack <archive> <parts> <cargo test flags…>
#       builds the test binaries (`cargo test --no-run`), splits them into
#       <parts> groups of about the same number of tests and writes them,
#       with the list, to <archive> (.tar.zst)
#   tools/ci/test-archive.sh run <archive> <part>
#       unpacks <archive> and runs the binaries of group <part> (1-based),
#       each from its crate's directory as `cargo test` would; runs them all
#       and fails at the end if any failed
#
# Why not cargo-nextest: it runs every test in a process of its own, and the
# testkit prepares a database (25 migrations, a ClickHouse schema) once per
# process. That is once per test binary under `cargo test`, and would be once
# per test, thousands of databases, under nextest.
#
# The checkout must be at the same path on every machine (it is on GitHub's
# runners): the tests find migrations and fixtures through paths compiled in.
set -euo pipefail
cd "$(dirname "$0")/../.."
root="$PWD"

pack() {
  local archive="$1" parts="$2"; shift 2
  local work; work="$(mktemp -d)"
  (cd backend && cargo test --no-run --message-format=json-render-diagnostics "$@") \
    | jq -r 'select(.reason == "compiler-artifact" and .executable != null)
             | "\(.profile.test)\t\(.manifest_path | sub("/Cargo.toml$"; ""))\t\(.executable)"' >"$work/artifacts"
  mkdir "$work/bin" "$work/programs"
  sed -n 's/^true\t//p' "$work/artifacts" >"$work/binaries"
  # The crates' programs too, where they were built: integration tests run them
  # through the path cargo compiled in (CARGO_BIN_EXE_<name>).
  sed -n 's/^false\t[^\t]*\t//p' "$work/artifacts" | sort -u >"$work/bin/programs.list"
  while read -r exe; do cp "$exe" "$work/programs/$(basename "$exe")"; done <"$work/bin/programs.list"
  mv "$work/programs" "$work/bin/"
  # Each binary under a name of its own, with its crate's directory and number of tests.
  while IFS=$'\t' read -r dir exe; do
    name="$(basename "$exe")"
    cp "$exe" "$work/bin/$name"
    tests="$(cd "$dir" && "$exe" --list --format terse 2>/dev/null | grep -c ': test$' || true)"
    printf '%s\t%s\t%s\n' "$tests" "${dir#"$root"/}" "$name"
  done <"$work/binaries" | sort -rn >"$work/weighted"
  # Largest first, each to the group with the fewest tests so far.
  awk -F'\t' -v parts="$parts" '{
      best = 1; for (p = 2; p <= parts; p++) if (load[p] < load[best]) best = p
      load[best] += $1; print best "\t" $2 "\t" $3
    } END { for (p = 1; p <= parts; p++) printf "group %d: %d tests\n", p, load[p] > "/dev/stderr" }' \
    "$work/weighted" >"$work/bin/groups"
  tar -C "$work/bin" -cf - . | zstd -T0 -3 --long=30 -q -o "$archive"
  echo "$(wc -l <"$work/binaries") test binaries, $(du -h "$archive" | cut -f1) archived"
  rm -rf "$work"
}

run() {
  local archive="$1" part="$2"
  local bin; bin="$(mktemp -d)"
  zstd -dc --long=30 "$archive" | tar -C "$bin" -xf -
  while read -r exe; do
    mkdir -p "$(dirname "$exe")" && cp "$bin/programs/$(basename "$exe")" "$exe"
  done <"$bin/programs.list"
  local failed=()
  while IFS=$'\t' read -r group dir name; do
    [ "$group" = "$part" ] || continue
    echo "::group::$dir $name"
    if (cd "$dir" && CARGO_MANIFEST_DIR="$root/$dir" "$bin/$name"); then
      echo "::endgroup::"
    else
      echo "::endgroup::"
      echo "::error::$dir: $name failed"
      failed+=("$dir $name")
    fi
  done <"$bin/groups"
  rm -rf "$bin"
  if [ "${#failed[@]}" -gt 0 ]; then
    printf 'FAILED: %s\n' "${failed[@]}" >&2
    exit 1
  fi
}

case "${1:-}" in
  pack) shift; pack "$@" ;;
  run) shift; run "$@" ;;
  *) sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit 1 ;;
esac
