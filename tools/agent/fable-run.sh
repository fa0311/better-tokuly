#!/usr/bin/env bash
# Runs one Fable 5.2 task on the prepaid API key, in its own worktree, and pushes the result.
#   fable-run.sh <name> <base ref> <budget usd> <task file>
# base ref: origin/main for new work, or origin/<pr branch> for a review-with-fixes (the task file
# then says to commit fixes onto that branch). The prompt is ~/tokuly-dev/notes/api-runs/common.md +
# the task file. Run it as a background command; its last lines are the report.
set -euo pipefail
name=$1 base=$2 budget=$3 task=$4
repo=$(git rev-parse --show-toplevel)
wt=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/wt/$name
notes=${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/notes/api-runs
git -C "$repo" fetch -q origin
branch=api/$name
case "$base" in origin/main) ;; *) branch=${base#origin/} ;; esac
if [ ! -d "$wt" ]; then
  if [ "$branch" = "api/$name" ]; then git -C "$repo" worktree add -q -b "$branch" "$wt" "$base"
  else git -C "$repo" worktree add -q -B "$branch" "$wt" "$base"; fi
fi
cat "$notes/common.md" "$task" > "$notes/$name.prompt"
"$(dirname "$0")/claude-api.sh" "$wt" claude-fable-5-2 "$budget" "$notes/$name.prompt" > "$notes/$name.report" 2>&1 || true
# The run may end before pushing (budget, crash): push whatever it committed.
if [ -n "$(git -C "$wt" log --oneline "origin/$branch..$branch" 2>/dev/null || git -C "$wt" log --oneline "$base..$branch")" ]; then
  git -C "$wt" push -q -u origin "$branch" 2>&1 | grep -v '^remote:' || true
fi
[ -n "$(git -C "$wt" status --porcelain | grep -v '^??')" ] && echo "NOTE: $wt has uncommitted changes"
tail -c 4000 "$notes/$name.report"
echo; echo "spent: $(tail -1 "${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/secrets/api-ledger.tsv" | cut -f3) USD; worktree $wt (remove it after merging)"
