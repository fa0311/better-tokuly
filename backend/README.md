# Tokuly backend

Rust workspace. One binary (`tokuly`, crate `crates/server`) runs every role:
`tokuly api`, `tokuly worker`, `tokuly migrate`, `tokuly all`.

## Layout

```
proto/tokuly/<domain>/v1/<domain>.proto   API contract (repo root), one package per domain
backend/migrations/NNNN_<domain>.sql      schema, one numbered file per domain
backend/crates/proto                      generated messages + Connect service traits (do not edit)
backend/crates/core                       shared foundation (see below)
backend/crates/testkit                    integration test harness
backend/crates/<domain>                   one crate per domain
backend/crates/server                     the binary; wires domains together
```

Domains and their migration numbers:

| crate | migration | depends on (crates) |
|---|---|---|
| core | 0001 | — |
| identity | 0100 | media |
| social | 0200 | identity |
| geo | 0300 | social |
| media | 0400 | — |
| stamp | 0500 | identity, media |
| chat | 0600 | social, stamp, media |
| notes | 0700 | social, stamp, media |
| video | 0800 | social, media |
| live | 0900 | video, wallet, stamp |
| shorts | 1000 | video |
| engagement | 1100 | identity, stamp |
| notify | 1200 | identity |
| feed | 1300 | identity |
| wallet | 1400 | identity |
| search | 1500 | identity, social |
| admin | 1600 | identity |
| emoji | 1700 | identity, media |
| ask | 1800 | social, media, stamp, wallet |
| quest | 1900 | social, wallet |
| plans | 2000 | social |
| school | 2100 | social |
| spaces | 2200 | social |
| community | 2300 | social, media, stamp |
| dev | 2400 | identity |
| realtime | — | identity |

A crate may only depend on crates listed for it (plus `core`, `proto`,
`identity`). Everything else goes through events (below), so the graph stays
acyclic and domains stay replaceable.

## Running

```sh
tools/dev/infra.sh up                 # Postgres, Valkey, NATS, RustFS, ClickHouse, Meilisearch, Mailpit, LiveKit
eval "$(tools/dev/infra.sh env)"      # TOKULY_* variables for that infrastructure
cd backend
cargo run -p tokuly-server -- all     # migrations + API + workers
cargo test -p tokuly-<domain>         # needs the infrastructure and the env above
```

`buf` must be on PATH (the proto crate shells out to `buf build`).

## API conventions (proto)

- `proto3`, package `tokuly.<domain>.v1`, file `proto/tokuly/<domain>/v1/<domain>.proto`.
  `buf lint` and `buf build` must stay clean (run from the repo root).
- IDs are strings holding UUIDs. Shareable content also has an 11-char
  `short_id`; RPCs accept either (`tokuly_core::ids::Ref`).
- Times are `google.protobuf.Timestamp`; durations are `*_seconds` / `*_ms` ints.
- Lists take `tokuly.common.v1.PageRequest page` and return `PageInfo page`.
- Reuse the shared types in `tokuly/common/v1/common.proto`: `UserSummary`,
  `ChannelSummary`, `Author`, `Image`, `Visibility`, `SubjectRef`, `StampRef`,
  `LatLng`; and `tokuly.media.v1.Attachment` / `LinkPreview` for media.
- Every RPC, message and non-obvious field gets a comment saying what it is
  for. Enum values are `TYPE_NAME_VALUE` with `..._UNSPECIFIED = 0`.
- Realtime pushes for a domain are a `oneof` inside its `<Domain>Event`
  message. The gateway carries it by its full name
  (`tokuly.realtime.v1.Event { type, payload }`), so `realtime.proto` does
  not import it: keep the name, and when a domain gets its first event
  message add it to the list in `realtime.proto` and to the table in
  `packages/realtime/src/events.ts`, which is how clients come to know it.
- Errors: Connect code plus a SCREAMING_SNAKE reason in the `x-tokuly-error`
  header. Document the reasons an RPC can return in its proto comment.

## Writing a domain

```rust
pub struct Chat(Arc<ChatInner>);            // cheap Clone, Deref to inner
impl Chat {
    pub fn new(core: Core, identity: Identity, cfg: ChatConfig) -> anyhow::Result<Self>;
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router;   // add every service
    pub fn spawn_workers(&self);            // event consumers, jobs, timers (only called in worker processes)
}
```

Domain settings are a `#[derive(clap::Args)] struct ChatConfig` with
`#[arg(long, env = "TOKULY_...", default_value = ...)]` fields and a
`Default` impl; the server flattens it into its CLI. See
`crates/identity/src/config.rs`.

All domain configs are flattened into one command line, so argument IDs must
be unique across domains: give a setting whose field name could collide an
explicit `id = "<domain>_<field>"` (the server's `command_line_is_consistent`
test catches clashes).

The server crate wires domains in `crates/server/src/app.rs`.

### Handlers

```rust
use connectrpc::{RequestContext, ServiceRequest, ServiceResult};
use tokuly_core::prelude::*;        // Core, AppError, AppResult, handle, ok, bail!, ensure!, parse_id, ts, opt_ts, Page, Uuid, CtxExt...

impl ChatService for Chat {
    async fn get_thing(&self, ctx: RequestContext, request: ServiceRequest<'_, GetThingRequest>) -> ServiceResult<GetThingResponse> {
        handle(async {
            let user_id = ctx.user_id()?;                       // or ctx.maybe_user_id()? for public RPCs
            let thing_id = parse_id(request.thing_id)?;         // views expose &str fields directly
            ensure!(self.can_see(user_id, thing_id).await?, not_found, "THING_NOT_FOUND", "thing not found");
            ok(GetThingResponse { thing: self.load(thing_id).await?.into(), ..Default::default() })
        })
        .await
    }
}
```

- Always wrap the body in `handle(async { ... }).await` and end with `ok(...)`:
  inside it `?` works on `sqlx`, cache, bus and any other error.
- `request` derefs to a zero-copy view (`&str`, `Option<&str>`,
  `MessageFieldView`, `RepeatedView`). For requests with nested messages,
  oneofs or when you need owned data call `let req = request.to_owned_message();`.
- Message fields: `MessageField<T>` — build with `value.into()` /
  `Some(value).into()`, read with `.as_option()`. Enums: `Kind::KIND_A.into()`,
  read with `.as_known()` / `.to_i32()`; an `i32` from the database converts
  with `.into()`. Always end struct literals with `..Default::default()`.
- Errors: `AppError::{invalid, not_found, already_exists, denied,
  unauthenticated, precondition, rate_limited, unavailable, unimplemented}(REASON, message)`,
  or the `bail!(kind, "REASON", "text {x}")` / `ensure!(cond, kind, "REASON", "text")`
  macros. Never leak whether something private exists: prefer `not_found`.
- Authorization lives in the handler/domain code, not in clients. Check
  ownership on every mutation and visibility on every read.
- Rate-limit abuse-prone RPCs: `core.cache.rate_limit("key:{user}", max, window).await?`
  (a deployment can switch all of them off with `TOKULY_RATE_LIMITS=false` for
  browser tests and load tests). For per-address limits use `ctx.client().ip`:
  it is the TCP peer, or the client named in `X-Forwarded-For` when the peer
  is one of our proxies (`TOKULY_TRUSTED_PROXIES`, see `tokuly_core::proxies`).

### Database

- PostgreSQL 18. Primary keys `uuid PRIMARY KEY DEFAULT uuidv7()`; `timestamptz`
  everywhere; `created_at`/`updated_at` with the `set_updated_at()` trigger.
- One migration file per domain (`backend/migrations/NNNN_<domain>.sql`, plain
  SQL, no down migrations). While a domain is under construction edit its file
  in place; tests rebuild their database from the files every run.
- Reference users with `REFERENCES users (id) ON DELETE CASCADE` (accounts are
  anonymized, not removed, so this only fires on hard cleanup). Do not put
  foreign keys on tables of domains you do not depend on.
- Another domain's tables are off limits, with one exception: read-only
  joins and lookups on identity's `users` table (names, status, deleted /
  banned checks) are fine where a function call per row would be wasteful.
  Never write to it or add indexes, functions or triggers on it.
- Queries are runtime-checked `sqlx::query_as::<_, Row>` / `query_scalar` with
  `#[derive(sqlx::FromRow)]` rows. Static SQL strings only; when a query must
  be assembled use `sqlx::QueryBuilder` or `sqlx::AssertSqlSafe(format!(...))`
  with **no user input in the string** (bind everything).
- `citext` columns must be selected as `col::text`.
- Structured values that mirror API messages are stored as proto JSON in
  `jsonb` (`serde_json::to_value(&msg)` / `serde_json::from_value`), e.g.
  images. Give tables an `attrs jsonb NOT NULL DEFAULT '{}'` column where
  future per-row extras are likely.
- Keyset pagination with `Page` (`page.fetch()` rows, `page.finish(&mut rows, key)`).
- Multi-step changes run in one transaction (`core.db.begin()`), with events
  emitted on the same transaction.

### Talking to other domains

Direct calls (only to crates you depend on):

- `tokuly_identity::{summaries, summaries_ordered, summary}` → `UserSummary`;
  `tokuly_identity::users::pick(&map, id)`; `tokuly_identity::settings::privacy_of`.
- `tokuly_media::assets::{require_ready, require_image_json, find_many}` to
  attach uploads (checks ownership, purpose and processing state);
  `AssetRow::to_attachment()`.
- `tokuly_social::graph::*` for friendship, blocks and audience checks.

Everything else is asynchronous, through `tokuly_core`:

| Need | Use |
|---|---|
| Tell other domains something happened | `events::emit(&mut *tx, "domain.entity.verb", Some(actor), &payload)`; payload structs live in your crate's public `events` module |
| React to another domain | `events::consume(bus, "your-durable-name", &["identity.user.deleted"], opts, cancel, handler)` in `spawn_workers` |
| Notify a user (inbox + push) | `outbound::notify(&mut *tx, &NotificationRequest::new(user, "chat.message", title, body, "/chat/…").actor(me))` |
| Activity log / friends feed entry | `outbound::activity(&mut *tx, &ActivityEntry::new(user, "note.posted", ActivityAudience::Friends, summary))` |
| Make something searchable | `outbound::index(&mut *tx, SearchDoc { kind: "video".into(), … })` / `outbound::unindex` |
| Push to connected clients | `core.rt.notify_user(user, &event).await` / `notify_users` / `notify_topic("live:<id>", &event)`, where `event` is your `<Domain>Event` (see below) |
| Background work | `core.jobs.enqueue("media.transcode", &payload)`, handled with `core.jobs.run(...)` in `spawn_workers` |
| Product analytics | `core.analytics.track(AnalyticsEvent::new("video.view").user(..).subject("video", id))` |
| Comments/likes/reports on your content | register a `SubjectResolver` for your `SubjectType` in `core.subjects` |
| Personal data export | register a `DataExporter` in `core.exporters` |
| Extra profile fields / relationship | implement `tokuly_identity::ProfileExtension` |

Every domain must handle `identity.user.deleted` (payload `{ "user_id": … }`)
by deleting or anonymizing that user's data.

Realtime pushes: hand `core.rt` your own event message and nothing else. It
wraps the message in the gateway's envelope, and the clients unwrap it again.

```rust
let event = ChatEvent { event: Some(typing.into()), ..Default::default() };
core.rt.notify_users(member_ids, &event).await;       // every device of these users
core.rt.notify_topic("live:<id>", &event).await;      // whoever subscribed to the topic
core.rt.to_user(user_id, &event).await?;              // the same, but a failure is yours to handle
```

Any generated message works (`buffa::Message + buffa::MessageName`); clients
only understand the types listed in `realtime.proto`. The `notify_*` methods
log a failed publish instead of returning it: a push is an optimization on
top of state that is already committed. Code that follows the bus itself (a
streaming RPC) reads a frame with `frame.event::<ChatEvent>()`.

User-facing text created on the server (notification titles, activity
summaries) is always given in Japanese and English via `Localized::new(ja, en)`.

### Tests

Integration tests live in `crates/<domain>/tests/*.rs` and use the testkit:

```rust
let app = TestApp::start(&["0001", "0100", "0400", "0600"], |core, identity, router| {
    Chat::new(core.clone(), identity.clone(), ChatConfig::default()).unwrap().register(router)
}).await;
let alice = app.user("alice").await;
let res: SendMessageResponse = app.call("tokuly.chat.v1.ChatService/SendMessage", Some(&alice), &req).await.unwrap();
let err = app.call::<_, SendMessageResponse>(..., Some(&bob), &req).await.unwrap_err();
assert_eq!(err.reason, "NOT_A_MEMBER");
```

- List exactly the migrations you need (yours + dependencies).
- All tests in one binary share a database: create fresh users per test.
- `app.events_by("chat.message.", alice.id)` inspects emitted events;
  `app.stream(...)` reads server-streaming RPCs.
- `app.listen_user(id)` / `listen_topic("live:<id>")` capture what is pushed
  (listen *before* the action). Read it as your event type:
  `pushes.next_event::<ChatEvent>(wait).await` (the next one, skipping other
  frames), `pushes.events::<ChatEvent>(window).await` (all of them), or
  `frame.event::<ChatEvent>()` on a frame from `pushes.next(wait)` /
  `pushes.drain(window)` when the envelope or the order of frames matters.
- Test event consumers by calling the handler function with a constructed
  `Event`; no relay runs in tests.
- Cover every RPC: the happy path, authorization (someone else's data,
  anonymous), and the validation errors with their reasons. Pure logic gets
  unit tests next to the code.

## Checklist before a domain is done

- `buf lint` (repo root) is clean; `cargo build -p tokuly-<domain>` has no warnings.
- `cargo test -p tokuly-<domain>` passes against the dev infrastructure.
- Events emitted/consumed and notification kinds are listed in `docs/events.md`
  under the domain's heading.
