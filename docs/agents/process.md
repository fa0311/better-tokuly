# How the work flows

The lead agent keeps the project moving; implementation is mostly done by delegated agents in their own
worktrees. The other agent (`agent:mac`) works through issues and pull requests (`docs/collaboration.md`).

## Every hour

`tools/agent/watchdog.sh`, then act on what it shows:
- new issues and comments — check the author first; give each issue an owner label, a priority label and,
  if someone asked something, an answer;
- pull requests ready for review — review (below), then `tools/agent/merge-pr.sh <pr>`;
- failed runs on `main` — one issue per failure, fixed in batches;
- stuck agents — every running delegate thread whose last update is over 30 minutes old: read its last
  items; if it hangs on a command, restart it in its own thread (`t3_thread_send`, mode `restart`) with
  what to do next, instead of starting over;
- idle capacity — fewer delegates running than the machine allows: start the next issues by priority.

## Delegating

One issue per agent, in its own worktree and build directory. The brief names the issue, the files to read,
the decisions already made, the checks, and the report format (last line `PR <url>, head <sha>, …`).
Agents must run long commands in the foreground: a background command's completion does not wake them.
Briefs for the common kinds of work: [`delegate-brief.md`](delegate-brief.md).

## Reviewing

Every PR gets an independent review before merge ([`review.md`](review.md)); risky ones (auth, money,
location, privacy, minors) by the strongest model available, with fixes committed onto the PR. A second,
narrower re-check after fixes for those risky ones. Verify claims yourself where cheap (checksums, a test).

## Releasing

Tag only when CI, Stack and End to end are green on the commit (`gh workflow run e2e.yml --ref main` when a
backend-only change skipped it); `release.yml` publishes images and archives.
