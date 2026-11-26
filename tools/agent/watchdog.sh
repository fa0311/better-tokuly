#!/usr/bin/env bash
# One look at everything the hourly watchdog checks; prints only what needs attention plus a summary.
#   watchdog.sh
set -uo pipefail
repo=fa0311/better-tokuly
cd "$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)" || exit 1
state=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/run/watchdog.since
since=$(cat "$state" 2>/dev/null || date -u -d '2 hours ago' +%FT%TZ)
date -u +%FT%TZ > "$state"

echo "== env-check"; "$(dirname "$(readlink -f "$0")")/machine/env-check.sh" | grep -E 'FAIL|disk free' || true
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
