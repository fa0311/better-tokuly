#!/usr/bin/env bash
# Merges a pull request the way this project does (docs/collaboration.md): marks it ready,
# squash-merges the exact head that was reviewed, and only then deletes its branch.
#   merge-pr.sh <pr> [--merge]        --merge: a merge commit (for fix PRs into a feature branch)
# Fails without touching anything when the PR conflicts or moved since it was read.
set -euo pipefail
pr=$1; method=squash; [ "${2:-}" = --merge ] && method=merge
repo=fa0311/better-tokuly
read -r state draft mergeable head branch < <(gh pr view "$pr" --repo "$repo" \
  --json state,isDraft,mergeable,headRefOid,headRefName \
  --jq '"\(.state) \(.isDraft) \(.mergeable) \(.headRefOid) \(.headRefName)"')
[ "$state" = OPEN ] || { echo "#$pr is $state" >&2; exit 1; }
[ "$draft" = true ] && gh pr ready "$pr" --repo "$repo" >/dev/null
for _ in 1 2 3 4 5; do   # GitHub computes mergeability lazily
  [ "$mergeable" != UNKNOWN ] && break
  sleep 3; mergeable=$(gh pr view "$pr" --repo "$repo" --json mergeable --jq .mergeable)
done
[ "$mergeable" = MERGEABLE ] || { echo "#$pr is $mergeable: rebase it first; nothing was changed" >&2; exit 1; }
gh api -X PUT "repos/$repo/pulls/$pr/merge" -f merge_method="$method" -f sha="$head" --jq .message
# Only after a successful merge: stacked PRs move onto main first (GitHub closes a PR whose base
# branch is deleted), then the branch is not needed any more.
for child in $(gh pr list --repo "$repo" --base "$branch" --json number --jq '.[].number'); do
  gh api -X PATCH "repos/$repo/pulls/$child" -f base=main --jq '"#\(.number) now targets \(.base.ref)"'
done
gh api -X DELETE "repos/$repo/git/refs/heads/$branch" >/dev/null 2>&1 || true
git -C "$(git -C "$(dirname "$(readlink -f "$0")")" rev-parse --show-toplevel)" pull -q --ff-only 2>/dev/null || true
echo "merged #$pr ($head) and deleted $branch"
