#!/usr/bin/env bash
# Adds images to a pull request's description, hosted by GitHub (user-attachments).
#   pr-images.sh <pr> <png>...        alt text = file name without extension
# When you open the PR, `gh pr create --attach <png>` is enough. This is for adding images later:
# `gh pr edit --attach` needs a token with read:org, so the images are uploaded through a comment,
# appended to the description with the REST API, and the comment is deleted again.
# The repository is public: only test data may be visible in the images.
set -euo pipefail
pr=$1; shift
repo=$(gh repo view --json nameWithOwner --jq .nameWithOwner)
args=()
for f in "$@"; do n=${f##*/}; args+=(--attach "$f#${n%.*}"); done
url=$(gh pr comment "$pr" --repo "$repo" --body "images for #$pr" "${args[@]}" | tail -1)
cid=${url##*-}
images=$(gh api "repos/$repo/issues/comments/$cid" --jq .body | grep -F '](https://github.com/user-attachments/')
body=$(gh api "repos/$repo/pulls/$pr" --jq .body)
gh api -X PATCH "repos/$repo/pulls/$pr" -f body="$body

$images" --jq .html_url
gh api -X DELETE "repos/$repo/issues/comments/$cid"
