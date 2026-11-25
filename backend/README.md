# Tokuly backend

Rust workspace. One binary (`tokuly`, crate `crates/server`) runs every role:
`tokuly api`, `tokuly worker`, `tokuly migrate`, `tokuly all`.

## Layout

```
proto/tokuly/<domain>/v1/<domain>.proto   API contract (repo root), one package per domain
backend/migrations/NNNN_<domain>.sql      schema, one numbered file per domain
backend/crates/proto                      generated messages + Connect service traits, one Cargo feature per package
backend/crates/core                       shared foundation (see below)
backend/crates/testkit                    integration test harness
backend/crates/<domain>                   one crate per domain
backend/crates/server                     the binary; wires domains together
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
| notes | 0700 | social, stamp, media |
| video | 0800 | social, media |
| live | 0900 | social, media, stamp, wallet, video |
| shorts | 1000 | social, media, video |
| engagement | 1100 | identity, social, media, stamp |
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
packages it names in its own code, and in `[dev-dependencies]` the ones only
its tests name:

```toml
[dependencies]
tokuly-proto = { workspace = true, features = ["chat", "common", "media"] }

[dev-dependencies]
tokuly-proto = { workspace = true, features = ["social"] }
```

So `cargo test -p tokuly-wallet` compiles `common`, `wallet` and what `core`,
`identity` and the testkit need, not the other twenty packages, and a change
to another domain's `.proto` does not rebuild it at all. The price is one
error to know: an unresolved import of `tokuly_proto::chat` means the `chat`
feature is missing from your `Cargo.toml`.

When a `.proto` starts importing another package, add that package to its
feature in `backend/crates/proto/Cargo.toml` (the build says so, and
`cargo test -p tokuly-proto` checks the whole table). A new package needs a
feature there and an alias in `backend/crates/proto/src/lib.rs`. The
generated Connect clients are behind the `client` feature, which nothing
enables today.

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

#### Declaring a setting

A setting is one such argument, and nothing else needs writing by hand: it
becomes a key of `tokuly.toml`, a row of the staff console's 「サーバー設定」
and an entry of `tokuly.example.toml` (`tokuly_core::settings`). Per argument:

- an `env = "TOKULY_<DOMAIN>_<NAME>"` and a `long` flag. The key is the
  variable without `TOKULY_` and the section prefix: `TOKULY_MEDIA_MAX_VIDEO_HOURS`
  is `[media] max_video_hours` (a variable without the prefix keeps its whole name);
- a doc comment: it is the description in the console and the example file
  (the server test `every_setting_has_a_description` fails without one);
- a value with a type: `bool` with `action = clap::ArgAction::Set` (never a
  bare flag), integers, `f64`, text, or a list with `value_delimiter = ','`;
  `value_parser = [..]` gives the console a choice;
- `hide_env_values = true` for anything secret (keys, passwords, tokens,
  URLs with credentials): the console then only writes it and shows the last
  four characters, the export leaves it out (`credentials_are_secrets` checks
  the obvious names).

**The console changes only what the domain allows.** A setting is set in
`tokuly.toml` or the environment unless its domain lists its key in
`<YourConfig>::CONSOLE_EDITABLE` (next to the struct, in `config.rs`). The
list is an allowlist on purpose: a new setting starts out of the console, so
adding one can never open a hole in it. List limits, quotas, retention,
intervals, feature tuning and texts. Never list:

- addresses the server talks to or is reached at, and origins or proxies it
  trusts (URLs, CORS / allowed origins, trusted proxies, LiveKit, MediaMTX);
- egress and SSRF switches (`*_allow_loopback`, `*_allow_private`,
  `*_allow_insecure`, allowed host lists);
- what the server executes or writes to (executable paths, directories);
- infrastructure names (index prefixes, keyspaces) and credentials of the
  infrastructure;
- protections and money: the rate limits switch, the minimum age, fees,
  exchange rates, coin rewards, the wallet's dev provider;
- privacy floors (minimum group sizes of anonymous or aggregate features).

An admin account is one password away from all of this, and a mistake in
these would let the console turn the server against its own network or
users. The server test `the_console_edits_only_what_domains_allow` names
the dangerous ones that must stay out.

What clap does not know goes on the section's `SectionSpec` in
`crates/server/src/settings.rs`:

- `.editable(YourConfig::CONSOLE_EDITABLE)`: the allowlist above;

- `.live(&["key", ..])`: the domain reads the value per use through
  `core.settings.current::<YourConfig>("section")` (see `Media::live`), so a
  console change applies at once. Everything else applies on restart and the
  console says so;
- `.managed(&[(prefix, path)])`: edited on a screen of its own, which the
  settings page links to;
- `.check(check!(YourConfig))`: the domain's `validate()`, run on a console
  change before it is saved (with every console value of the section), and
  at start-up and on every reload: console values a section fails with are
  set aside, so a console mistake never stops the server. Make `validate()`
  build what the domain builds from its settings without the network, as
  `Config::validate` builds the mailer.

After adding or changing a setting, regenerate the example file from the
repository root: `backend/target/.../tokuly config example > tokuly.example.toml`
(`cargo run -p tokuly-server -- config example`); the server test
`the_example_file_is_current` fails until it matches.

Precedence, highest first: command line and environment, `tokuly.toml`
(`TOKULY_CONFIG`), the console (table `server_settings`, history in
`server_setting_changes`), the default. An environment variable that is set
but empty counts as not set (env files are full of `TOKULY_X=` lines); the
start warns when such a line would have hidden another value. Where empty
means something ("no proxies", "bonus off", "omit the parameter"), list the
key in `<YourConfig>::EMPTY_IS_A_VALUE` and pass it to
`.empty_is_value(..)`: an empty variable is then that value. The server test
`documented_empty_values_are_kept` fails for a text or list setting with a
non-empty default whose description mentions an empty value but is not
listed.

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
access ends later, end the subscriptions too, after the change is
committed: `core.rt.revoke(user, topic)` / `revoke_prefix(user, "live")`
(that user's subscriptions are checked against the guard again on every
gateway and dropped where it refuses — harmless when nothing changed, so
call it on anything that *might* end access: bans, removals, leaving,
blocks and unfriendings), `core.rt.recheck_topic(topic)` (every subscriber,
for an audience narrowed to members) and `core.rt.close_topic(topic)`
(everyone, for a deleted room or an ended share). See
`docs/events/realtime.md`, "Ending subscriptions". The `notify_*` methods
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
