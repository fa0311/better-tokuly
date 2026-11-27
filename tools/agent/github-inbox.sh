#!/usr/bin/env bash
# Show unread GitHub discussion notifications for this repository.
# This is an on-demand inbox view; it never marks notifications as read.
set -euo pipefail

repo="fa0311/better-tokuly"

notifications=$(gh api --paginate notifications)

printf '%s\n' "$notifications" | jq -s -r --arg repo "$repo" '
  (add // [])
  | map(select(
      .repository.full_name == $repo
      and .unread == true
      and (
        .reason == "mention"
        or .reason == "team_mention"
        or .reason == "review_requested"
        or .reason == "assign"
        or .reason == "comment"
        or .reason == "author"
        or .reason == "subscribed"
        or .reason == "manual"
      )
    ))
  | sort_by(.updated_at)
  | reverse
  | if length == 0 then
      "No unread discussion notifications for \($repo)."
    else
      .[]
      | . as $notification
      | (if $notification.subject.type == "PullRequest" then "pull" else "issues" end) as $kind
      | ($notification.subject.url | split("/")[-1]) as $number
      | "[\($notification.reason)] \($notification.subject.title)  \($notification.updated_at)\nhttps://github.com/\($repo)/\($kind)/\($number)"
    end
'
