# What every delegated agent is told

Put this, plus the issue and the decisions already made, in the brief. Keep the brief itself short.
Copy the first rule into the brief word for word: a link to it is not enough, agents forget it.

- **Foreground only, never end your turn before the report**: never `run_in_background`; nothing wakes
  you after your turn ends. Split anything longer than ten minutes into calls of at most 600000 ms; to
  wait for something already running, loop in the foreground (`timeout 590 sh -c 'while pgrep -f gradle
  >/dev/null; do sleep 30; done'`, repeated).

- **Read first**: `CLAUDE.md`, the README of the area, `docs/collaboration.md`, the docs of the domain
  (`docs/events/<domain>.md`), and for backend work `docs/agents/backend-checklist.md` (binding).
- **Style**: match the code, READMEs and docs as of commit `060b8f66`. READMEs and docs don't grow;
  new detail goes into `docs/` at the same density.
- **Your own checkout**: a worktree and a build directory `$TOKULY_DEV_HOME/target/<name>`;
  `CARGO_INCREMENTAL=0`; build and test only the crates you touch plus `tokuly-server`; delete
  `debug/incremental` and the directory when done. Never `cargo clean` anything else.
- **Keep the existing rules** of the area you extend: audiences and blocks both ways, withdrawal events,
  rate limits, the egress guard, session revocation, account safety on mobile (`apps/mobile/README.md`).
  Classify every new RPC in `backend/crates/core/src/api_access.rs` and the dev scope catalogue.
- **Web and mobile in the same PR** as the backend change they need; Japanese first.
- **Checks**: build without warnings, the touched crates' tests, `buf lint`, web/mobile lint, typecheck and
  tests, the browser or Maestro tests of the flows you changed (local emulator: `tools/agent/android.sh`).
  Do not wait for GitHub Actions.
- **PR**: Draft with `agent:*` and area labels, `Fixes #n`; screenshots if the UI changed
  (`gh pr create --attach`, later `tools/agent/pr-images.sh`); when local checks pass, `gh pr ready` and `needs-review`.
- **Report** (your last message, the lead reads only this): what changed, what you checked, what is not done
  and why; last line `PR <url>, head <sha>, checks and reviews <state>`.
