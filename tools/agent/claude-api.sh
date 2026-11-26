#!/usr/bin/env bash
# Runs headless Claude Code billed to an API key kept in $TOKULY_DEV_HOME/secrets/anthropic-api-key (mode 600, never in the repo),
# separate from the subscription the T3 Code agents use. Keeps a ledger and refuses to start past the cap.
#   claude-api <workdir> <model> <budget-usd> <prompt-file>
# The key never appears in arguments, logs or the repo.
set -euo pipefail
LEDGER="${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/secrets/api-ledger.tsv"   # date \t model \t usd \t workdir
CAP=${CLAUDE_API_CAP_USD:-198}                      # of the $200 credit (user: use it all; no extra billing)
dir=$1 model=$2 budget=$3 prompt=$4
spent=$(awk -F'\t' '{s+=$3} END {printf "%.2f", s+0}' "$LEDGER" 2>/dev/null || echo 0)
if awk -v s="$spent" -v b="$budget" -v c="$CAP" 'BEGIN{exit !(s+b>c)}'; then
  echo "claude-api: \$$spent spent, \$$budget more would pass the cap \$$CAP" >&2; exit 3
fi
out=$(mktemp)
cd "$dir"
# A clean environment: inside a T3/Claude Code session the CLAUDE_CODE_* variables route a child
# claude through the parent's login, which ignores the API key ("Credit balance is too low").
env -i HOME="$HOME" PATH="$PATH" TERM=dumb LANG=C.UTF-8 GITHUB_TOKEN="${GITHUB_TOKEN:-}" GH_TOKEN="${GITHUB_TOKEN:-}" \
  ANTHROPIC_API_KEY="$(cat "${TOKULY_DEV_HOME:-$HOME/tokuly-dev}/secrets/anthropic-api-key")" \
  claude -p --model "$model" --max-budget-usd "$budget" --permission-mode bypassPermissions \
         --output-format json < "$prompt" > "$out" || true
cost=$(python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print(d.get('total_cost_usd',0))" "$out" 2>/dev/null || echo "$budget")
printf '%s\t%s\t%s\t%s\n' "$(date -u +%FT%TZ)" "$model" "$cost" "$dir" >> "$LEDGER"
python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print(d.get('result',''))" "$out" 2>/dev/null || cat "$out"
rm -f "$out"
