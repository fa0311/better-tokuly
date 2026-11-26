# Tokuly

Agents: start with [`AGENTS.md`](AGENTS.md) (decisions, process, tools, machine).

Location sharing + chat + notes + video + live + shorts in one app (a rebuild
of the original Tokuly, modelled on Bump/Zenly). Monorepo:

- `proto/` — the API contract (Protobuf, Connect RPC). Source of truth for every client and the backend.
- `backend/` — Rust workspace, one crate per domain. **Read `backend/README.md` before touching backend code.**
- `apps/web` (TanStack Start), `apps/mobile` (Expo), `apps/studio` (Tauri) and shared `packages/*` — pnpm workspace.
- `infra/`, `compose.yaml` — the Docker Compose stack.
- `docs/domains.md` — product scope per domain; `docs/events.md` — event catalogue.

## Working here

- Backend dev needs the native infrastructure: `tools/dev/infra.sh up && eval "$(tools/dev/infra.sh env)"`.
- `buf lint` (repo root) must stay clean. Never leave `proto/` in a state where `buf build` fails: every Rust build depends on it.
- Stay inside the domain you are working on: its proto file, its migration file, its crate. Other domains are reached through the documented cross-domain APIs and events, not by editing their code.
- UI copy is Japanese first (casual, friendly, exclamatory like the original: 「友達と位置情報共有を楽しもう!」), English second.
