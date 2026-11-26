#!/usr/bin/env bash
# Publishes screenshots for a pull request and prints the Markdown to paste into its body.
#   pr-shots <pr number> <image>...
# Images go to the orphan branch `pr-screenshots` under pr-<n>/ (main stays free of binaries).
# The repository is public: only demo/test data may be visible in the images.
set -euo pipefail
pr=$1; shift
repo=fa0311/better-tokuly
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
git -C "$work" init -q
git -C "$work" remote add origin "https://github.com/$repo.git"
if git -C "$work" fetch -q --depth 1 origin pr-screenshots 2>/dev/null; then
  git -C "$work" checkout -q -b pr-screenshots FETCH_HEAD
else
  git -C "$work" checkout -q --orphan pr-screenshots
fi
mkdir -p "$work/pr-$pr"
for f in "$@"; do cp "$f" "$work/pr-$pr/"; done
git -C "$work" add "pr-$pr"
git -C "$work" -c user.name="$(git config user.name)" -c user.email="$(git config user.email)" \
  commit -q -m "Screenshots for #$pr"
git -C "$work" push -q origin pr-screenshots
sha=$(git -C "$work" rev-parse HEAD)
for f in "$@"; do
  name=$(basename "$f")
  printf '![%s](https://raw.githubusercontent.com/%s/%s/pr-%s/%s)\n' "${name%.*}" "$repo" "$sha" "$pr" "$name"
done
