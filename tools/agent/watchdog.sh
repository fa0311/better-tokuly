#!/usr/bin/env bash
# One look at everything the hourly watchdog checks; prints only what needs attention plus a summary.
#   watchdog.sh
set -uo pipefail
repo="fa0311/better-tokuly"
script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)" || exit 1
repo_root="$(git -C "$script_dir" rev-parse --show-toplevel)" || exit 1
cd "$repo_root" || exit 1
state=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/run/watchdog.since
mkdir -p "$(dirname "$state")" || exit 1
since=$(cat "$state" 2>/dev/null || true)
if [ -z "$since" ]; then
  case "$(uname -s)" in
    Darwin) since=$(date -u -v-2H +%FT%TZ) ;;
    *) since=$(date -u -d '2 hours ago' +%FT%TZ) ;;
  esac
fi
case "$(uname -s)" in
  Linux)
    echo "== env-check"
    "$script_dir/machine/env-check.sh" | grep -E 'FAIL|disk free' || true
    ;;
  Darwin)
    echo "== macOS"
    printf 'macOS %s (%s)\n' "$(sw_vers -productVersion 2>/dev/null || echo unknown)" "$(uname -m)"
    free_gb=$(df -Pk "$HOME" | awk 'END { if (NF >= 4) printf "%d", $4 / 1024 / 1024 }')
    if [ -n "$free_gb" ]; then echo "disk free ${free_gb} GiB"; else echo "disk free unknown"; fi
    if xcode_path=$(xcode-select -p 2>/dev/null); then echo "Xcode: $xcode_path"; else echo "FAIL Xcode command-line tools are not selected"; fi
    ;;
  *)
    echo "== env-check"
    echo "skipped for unsupported operating system: $(uname -s)"
    ;;
esac
echo "== services"; tools/dev/infra.sh status 2>&1 | grep -v ' up ' || true
printf 'dev api %s\n' "$(curl -s -o /dev/null -w '%{http_code}' http://127.0.0.1:58081/healthz)"
# The shared API must not be older than main's last backend change (a stale one made #430 and #474 chase ghosts).
bin=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/target/main/debug/tokuly
changed=$(git log -1 --format=%ct origin/main -- backend proto 2>/dev/null || echo 0)
if [ -x "$bin" ] && [ "$(stat -c %Y "$bin")" -lt "$changed" ]; then
  echo "FAIL dev api binary is older than main's last backend change: rebuild (CARGO_TARGET_DIR=…/target/main cargo build -p tokuly-server) and restart (machine/dev.sh api)"
fi

# Capture the start of this check. Anything posted after this point will still
# be returned by the next run if it arrives while the requests are in flight.
checked_at=$(date -u +%FT%TZ) || { echo "FAIL could not read the current time; cursor unchanged" >&2; exit 1; }
tmp=$(mktemp -d /tmp/tokuly-watchdog.XXXXXX) || { echo "FAIL could not create a temporary directory; cursor unchanged" >&2; exit 1; }
state_tmp="$state.tmp.$$"
trap 'rm -rf "$tmp"; rm -f "$state_tmp"' EXIT

if ! gh pr list --repo "$repo" --json number,isDraft,headRefName,labels,title \
  --jq '.[]|"#\(.number) draft=\(.isDraft) \(.headRefName) [\([.labels[].name]|join(","))] \(.title)"' >"$tmp/open-prs"; then
  echo "FAIL GitHub open-PR query failed; cursor unchanged at $since" >&2
  exit 1
fi
if ! gh api --paginate "repos/$repo/issues?state=all&since=$since&per_page=100" \
  --jq ".[]|select(.created_at>=\"$since\")|\"#\(.number) \(.user.login) pr=\(.pull_request!=null) \(.title)\"" >"$tmp/issues"; then
  echo "FAIL GitHub issue query failed; cursor unchanged at $since" >&2
  exit 1
fi
if ! gh api --paginate "repos/$repo/issues/comments?since=$since&per_page=100" \
  --jq '.[]|"#\(.issue_url|split("/")|last) \(.user.login) \(.body[0:120]|gsub("\n";" "))"' >"$tmp/comments"; then
  echo "FAIL GitHub comment query failed; cursor unchanged at $since" >&2
  exit 1
fi
if ! gh api --paginate "repos/$repo/pulls/comments?since=$since&per_page=100" \
  --jq '.[]|"#\(.pull_request_url|split("/")|last) \(.user.login) [inline] \(.body[0:120]|gsub("\n";" "))"' >"$tmp/review-comments"; then
  echo "FAIL GitHub pull-request review comment query failed; cursor unchanged at $since" >&2
  exit 1
fi
if ! gh run list --repo "$repo" --branch main -L 20 --json name,status,conclusion,headSha \
  --jq 'group_by(.name)|map(.[0])|.[]|"\(.name) \(.status) \(.conclusion) \(.headSha[0:8])"' >"$tmp/main-ci"; then
  echo "FAIL GitHub CI query failed; cursor unchanged at $since" >&2
  exit 1
fi

# Advance only after every GitHub query succeeded. The timestamp represents
# the start of the check, so comments created during it are picked up next time.
if ! printf '%s\n' "$checked_at" >"$state_tmp" || ! mv -f "$state_tmp" "$state"; then
  echo "FAIL could not save watchdog cursor; previous cursor remains $since" >&2
  exit 1
fi

echo "== open PRs"; cat "$tmp/open-prs"
echo "== new issues since $since (author!)"; cat "$tmp/issues"
echo "== new comments since $since"; cat "$tmp/comments"
echo "== new inline review comments since $since"; cat "$tmp/review-comments"
echo "== main CI (latest per workflow)"; cat "$tmp/main-ci"
