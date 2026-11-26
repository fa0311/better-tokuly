# Tokuly backend

Rust workspace. One binary (`tokuly`, crate `crates/server`) runs every role:
`tokuly api`, `tokuly worker`, `tokuly migrate`, `tokuly all`.
(`tokuly demo` is the odd one out: the demo simulator, which only calls the
API over HTTP; see `crates/demo/src/lib.rs`.)

## Layout

```
proto/tokuly/<domain>/v1/<domain>.proto   API contract (repo root), one package per domain
backend/migrations/NNNN_<domain>.sql      schema, one numbered file per domain
backend/crates/proto                      generated messages + Connect service traits, one Cargo feature per package
backend/crates/core                       shared foundation (see below)
backend/crates/testkit                    integration test harness
backend/crates/<domain>                   one crate per domain
backend/crates/server                     the binary; wires domains together
backend/crates/demo                       the demo simulator (`tokuly demo`): a client of the API, not a domain
```

Domains and their migration numbers. The binary serves all of them
(`crates/server/src/app.rs`):

| crate | migration | depends on (crates) |
|---|---|---|
| core | 0001 | — |
| identity | 0100 | media |
| social | 0200 | identity |
| geo | 0300 | social, media |
| media | 0400 | — |
| stamp | 0500 | identity, media, wallet |
| chat | 0600 | social, stamp, media |
| notes | 0700 | social, stamp, media, emoji |
| video | 0800 | social, media |
| live | 0900 | social, media, stamp, wallet, video |
| shorts | 1000 | social, media, video |
| engagement | 1100 | identity, social, media, stamp, emoji |
| notify | 1200 | identity |
| feed | 1300 | identity, social |
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
| realtime | — | — |

A crate may only depend on crates listed for it (plus `core`, `proto`,
`identity`). Everything else goes through events (below), so the graph stays
acyclic and domains stay replaceable.

### API packages

`tokuly-proto` compiles only the proto packages a build asks for: every
package is a Cargo feature of the same name (`chat` is `proto/tokuly/chat`,
`tokuly_proto::chat`), and nothing is on by default. A crate lists the
packages its code names, and in `[dev-dependencies]` the ones only its tests
name; an unresolved import of `tokuly_proto::chat` means the feature is
missing from your `Cargo.toml`.

```toml
[dependencies]
tokuly-proto = { workspace = true, features = ["chat", "common", "media"] }

[dev-dependencies]
tokuly-proto = { workspace = true, features = ["social"] }
```

When a `.proto` starts importing another package, add that package to its
feature in `backend/crates/proto/Cargo.toml` (`cargo test -p tokuly-proto`
checks the table). A new package needs a feature there and an alias in
`backend/crates/proto/src/lib.rs`.

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
  (`tokuly.realtime.v1.Event { type, payload }`): keep the name. A first
  event message is registered in three places (see "Talking to other domains").
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

#### Declaring a setting

A setting is one such argument with an `env = "TOKULY_<DOMAIN>_<NAME>"`, a
`long` flag, a doc comment (its description in the console and in
`tokuly.example.toml`) and a typed value; `bool` takes
`action = clap::ArgAction::Set`, anything secret `hide_env_values = true`.
Regenerate the example file after a change
(`cargo run -p tokuly-server -- config example > tokuly.example.toml`).

The staff console changes only the keys a domain lists in
`<YourConfig>::CONSOLE_EDITABLE`. List limits, quotas, retention, intervals,
feature tuning and texts. Never list addresses and trusted origins or
proxies, egress and SSRF switches, executable paths and directories,
infrastructure names and credentials, OAuth and push identities, protections
and money (the rate limits switch, the minimum age, fees, rates, rewards),
or privacy floors: an admin account is one password away from the console.

Keys, live values, validation, precedence and empty values:
[`docs/backend/settings.md`](../docs/backend/settings.md).

The server crate wires domains in `crates/server/src/app.rs`: the config in
`DomainConfigs`, construction in `Domains::new` (in dependency order, and
before `Engagement` when the domain registers a subject resolver),
`register`, `spawn_workers`, and plain HTTP routes in `api_router` when the
domain has any. What a domain needs from one it must not depend on is a
small trait of its own that `crates/server/src/bridges.rs` implements. Every
new RPC also gets a line in `crates/core/src/api_access.rs` (what an API
token needs to call it; a test in `tokuly-dev` fails without it).

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
| Request a URL or host that comes from outside (previews, feeds, webhooks, push endpoints) | `tokuly_core::egress`: `check_url`, `client_builder(Some(policy), timeout)` (guarded resolver, no redirects, no proxy), `check_peer`, `read_capped` |
| Product analytics | `core.analytics.track(AnalyticsEvent::new("video.view").user(..).subject("video", id))` |
| Comments/likes/reports on your content | register a `SubjectResolver` for your `SubjectType` in `core.subjects` |
| Personal data export | register a `DataExporter` in `core.exporters` |
| Extra profile fields / relationship | implement `tokuly_identity::ProfileExtension` |

An event or job whose handler keeps failing is retried and then buried
(`tokuly dead-letters list` / `redrive`, `docs/events.md`). Return an error
from a handler only for something a retry can fix; for an event that can
never be handled (the row is gone, the payload makes no sense) log and
return `Ok(())`.

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

Only a domain's event message is accepted (`tokuly_proto::RealtimeEvent`):
passing a `ChatMessage`, or a value of the event's `oneof`, does not compile,
because no client would understand it. A domain that gets its first event
message adds it in three places, which tests keep in step: the list at
`Event` in `realtime.proto`, `backend/crates/proto/src/events.rs` and the
table in `packages/realtime/src/events.ts`.

A topic's guard (`core.topic_guards`) runs when someone subscribes. When
access may have ended (bans, removals, leaving, blocks, unfriendings), end
the subscriptions too, after the change is committed:
`core.rt.revoke(user, topic)` / `revoke_prefix(user, "live")`,
`core.rt.recheck_topic(topic)` for a narrowed audience,
`core.rt.close_topic(topic)` for a deleted room (`docs/events/realtime.md`,
"Ending subscriptions"). The `notify_*` methods log a failed publish instead
of returning it: a push is an optimization on top of state that is already
committed. Code that follows the bus itself (a streaming RPC) reads a frame
with `frame.event::<ChatEvent>()`.

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
