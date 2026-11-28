# How the work flows

The lead agent keeps the project moving; implementation is mostly done by delegated agents in their own
worktrees. The other agent (`agent:mac`) works through issues and pull requests (`docs/collaboration.md`).

## Every hour

`tools/agent/watchdog.sh`, then act on what it shows:
- new issues and comments — check the author first; give each issue an owner label, a priority label and,
  if someone asked something, an answer;
- pull requests ready for review — review (below), then `tools/agent/merge-pr.sh <pr>`;
- failed runs on `main` — one issue per failure, fixed in batches; look at the pass rate of End to end over
  its last finished runs, not only the latest: under half green, finding the causes comes first;
- stuck agents — every running delegate thread whose last update is over 30 minutes old: read its last
  items; if it hangs on a command, restart it in its own thread (`t3_thread_send`, mode `restart`) with
  what to do next, instead of starting over;
- idle capacity — fewer delegates running than the machine allows: start the next issues by priority,
  at least half of them new features; no delegate running is never "nothing to do".

## Delegating

One issue per agent, in its own worktree and build directory. The brief names the issue, the files to read,
the decisions already made, the checks, and the report format (last line `PR <url>, head <sha>, …`).
Agents must run long commands in the foreground: a background command's completion does not wake them.
Briefs for the common kinds of work: [`delegate-brief.md`](delegate-brief.md).

## Reviewing

Every PR gets an independent review before merge ([`review.md`](review.md)); risky ones (auth, money,
location, privacy, minors) by the strongest model available, with fixes committed onto the PR. A second,
narrower re-check after fixes for those risky ones. Verify claims yourself where cheap (checksums, a test), and
first whatever the author's report says was not checked.

## Stacked pull requests

A pure move of shared code goes first, the feature on top. Review and merge the move as soon as it is ready:
moves of several areas touch the same index files and conflict with each other while they wait. After the
squash merge, move the feature onto main with `git rebase --onto origin/main <old head of the move>`.

## Releasing

Tag only when CI, Stack and End to end are green on the commit (`gh workflow run e2e.yml --ref main` when a
backend-only change skipped it); `release.yml` publishes images and archives.
Before a stable tag (`vX.Y.Z`), freeze the migrations it ships: `cd backend/migrations && sha384sum
[0-9]*.sql > frozen.sha384`, commit; `release.yml` refuses a stable tag whose table is out of date or that
does not upgrade from the stable release before it. From the first stable tag on, schema changes follow
`backend/README.md`, "After a release".
