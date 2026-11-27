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
date -u +%FT%TZ > "$state"

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
echo "== open PRs"; gh pr list --repo $repo --json number,isDraft,headRefName,labels,title \
  --jq '.[]|"#\(.number) draft=\(.isDraft) \(.headRefName) [\([.labels[].name]|join(","))] \(.title)"'
echo "== new issues since $since (author!)"
gh api "repos/$repo/issues?state=all&since=$since&per_page=100" \
  --jq ".[]|select(.created_at>=\"$since\")|\"#\(.number) \(.user.login) pr=\(.pull_request!=null) \(.title)\""
echo "== new comments since $since"
gh api "repos/$repo/issues/comments?since=$since&per_page=100" \
  --jq '.[]|"#\(.issue_url|split("/")|last) \(.user.login) \(.body[0:120]|gsub("\n";" "))"'
echo "== main CI (latest per workflow)"
gh run list --repo $repo --branch main -L 20 --json name,status,conclusion,headSha \
  --jq 'group_by(.name)|map(.[0])|.[]|"\(.name) \(.status) \(.conclusion) \(.headSha[0:8])"'
