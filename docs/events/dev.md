# dev — events, jobs and configuration

Crate `tokuly-dev`, migration `2400_dev.sql`, API `tokuly.dev.v1`
(`TokenService`, `AppService`, `AuthorizationService`, `BotService`,
`WebhookService`, `ReferenceService`, `UsageService`). Depends on `identity`.

The developer platform: personal access tokens, OAuth 2.1 applications,
bots, webhooks, the API reference and usage statistics.

## How a token authenticates

```
Authorization: Bearer tkl_…  ──▶ core auth middleware (tokuly_core::api_tokens)
                                   │  verifier registered by this crate (Valkey, then PostgreSQL)
                                   ├─ rate limit (per credential and minute)
                                   ├─ scope of the RPC, from its path
                                   └─ AuthContext { user_id, roles: [], api_token: Some(…) } ──▶ handler
```

- Secrets are `tkl_<kind>_<36 random><6 checksum>`: `pat` personal access
  token, `bot` bot token, `oat` / `ort` OAuth access and refresh token, `oac`
  authorization code, `odc` device code, `cs` client secret, `whs` webhook
  signing secret. Only a keyed hash is stored (webhook secrets: encrypted).
- Scopes are `<domain>:read` / `<domain>:write`. What each RPC needs is
  listed RPC by RPC in `tokuly_core::api_access`; an RPC that is not listed
  cannot be called with a token (`TOKEN_NOT_ALLOWED`), and a test in this
  crate walks the descriptor set and fails for any RPC without a line. Some
  RPCs need a second scope: what carries location (`MapStoryService`, the
  activity feed, `GetProfile`, nearby discovery) also needs `geo:read`, the
  notification inbox also `chat:read` and `geo:read`, the caller's reports
  also `chat:read`, and whatever spends coins (`PurchasePack`, `BuyHint`)
  also `wallet:write`. Session only: signing
  in, credentials, linked services, account deletion and export, purchases
  and payouts, SOS, revealing an anonymous author, everything staff, and in this domain everything except
  reading webhooks, deliveries, usage and the reference.
- The catalogue with the wording shown to people is `tokuly_dev::scopes`; it
  offers exactly the scopes some listed RPC needs (no scopes yet for live,
  shorts, quest, spaces).
- A response that keeps streaming is cut off when its token is revoked or
  expires (re-verified every 30 s).
- A verified token is cached for up to 60 s (`dev:tok:<hash>`). Revocation,
  bans noticed through events, bot switches and application takedowns remove
  the entry, so they apply to the next request. A password change is noticed
  when the entry expires (see below).
- A token is refused while the account it acts as, or the person who made it,
  is suspended; an OAuth token also while the application is disabled or its
  owner is suspended. To everyone but its owner such an application does not
  exist (`CLIENT_NOT_FOUND`, `APP_NOT_FOUND`, `invalid_client`).
- Whatever may be used once is taken in one atomic step (authorization code,
  device approval, refresh token), and answers do not distinguish a secret
  that is unknown from one that is expired, spent or somebody else's.

## Events emitted

Payload structs: `tokuly_dev::events`.

| Topic | Payload | When |
|---|---|---|
| `dev.token.created` | `TokenEvent { token_id, user_id, kind, reason }` | A personal (`kind: "pat"`) or bot (`"bot"`) token was created. `user_id` is the account it acts as. Actor: the creator. |
| `dev.token.revoked` | `TokenEvent` | A token was revoked. `reason`: `owner`, `password_changed`, `banned`, `bot_deleted`. Actor: the owner, or none. |
| `dev.app.created` / `.updated` / `.deleted` | `AppEvent { app_id, owner_id }` | An OAuth application was registered, edited or deleted (deletion also when its owner's account is deleted; then without actor). |
| `dev.app.disabled` | `AppEvent` | Moderation took an application down. |
| `dev.authorization.granted` | `AuthorizationEvent { app_id, user_id, scopes, reason }` | A user approved an application (consent screen or device page). `scopes` are the ones just approved. Actor: the user. |
| `dev.authorization.revoked` | `AuthorizationEvent` | A user's authorization of an application was withdrawn. `reason`: `user`, `banned`. Not emitted for a token family revoked on its own (replayed code or refresh token), nor per user when an application is deleted. |
| `dev.bot.created` / `.updated` / `.deleted` | `BotEvent { bot_id, owner_id }` | A bot account was created, its profile edited, or it was deleted. |
| `dev.webhook.disabled` | `WebhookDisabled { webhook_id, account_id }` | A webhook was disabled because its deliveries kept failing. |

No event is emitted in identity's name: a bot is not announced as `identity.user.created`, and a bot's profile change not as `identity.user.updated`.

## Events consumed

| Topic | Durable | Effect |
|---|---|---|
| `identity.user.banned` | `dev-accounts` | Revokes every token the account created (its own and its bots'), every token acting as it, and its authorizations of applications — those that existed when the ban happened (`occurred_at`), so a late event does not touch what came after. Lifting the ban does not restore them. The tokens of applications the account owns are refused while it is suspended. |
| `identity.user.unbanned` | `dev-accounts` | Nothing is restored; what was not revoked (the tokens of the account's applications) is judged afresh at once instead of when the cached refusal expires. |
| `identity.user.password_changed` | `dev-accounts` | Revokes every personal and bot token the account created before the change (`occurred_at`). **Identity does not emit this yet**; until it does, the same happens when a token is next verified against the database (within 60 s of its next use): each token remembers a fingerprint of its creator's password hash, and the tokens made under another password than the current one are revoked. |
| `identity.user.deleted` | `dev-accounts` | Deletes the account's tokens, applications (with every user's authorization of them), the authorizations it gave, its webhooks with their logs, and its bots (whose accounts are then deleted through identity). For a bot account: the record of who owned it is kept (see "Bots"). |
| `admin.content.removed` | `dev-accounts` | `SUBJECT_TYPE_APP`: the application is disabled (`AppStatus.DISABLED`, `reason` shown to its owner), its tokens stop working at once, its owner is notified. Other subject types are ignored. |
| the webhook sources (below) | `dev-webhooks` | Stored as deliveries for the webhooks that subscribed. |

## Webhooks

A webhook belongs to an account (a person or a bot) and receives that
account's events. The catalogue is `tokuly_dev::webhooks::EVENT_TYPES`.
Creating, changing, deleting, pinging and replaying take a session; a token
with `dev:read` lists webhooks and reads only deliveries of event types
whose scopes it holds. A delivery goes to the webhook's
current URL, so a corrected address receives what was waiting.

| Event type | Source | Delivered to | Scopes to read its deliveries |
|---|---|---|---|
| `notification.created` | every `notify.requested` | the recipient | `notify:read`, `chat:read`, `geo:read` |
| `friend.request_received` | `social.friend.requested` | `to_user_id` | `social:read` |
| `friend.added` | `social.friend.accepted` | `user_id`, `friend_id` | `social:read` |
| `friend.removed` | `social.friend.removed` | `user_id` | `social:read` |
| `chat.message_received` | notification kind `chat.message` | the recipient | `chat:read` |
| `chat.mentioned` | notification kind `chat.mention` | the recipient | `chat:read` |
| `chat.call_incoming` | notification kind `chat.call` | the recipient | `chat:read` |
| `place.alert` | notification kind `geo.place_alert` | the recipient | `geo:read` |
| `channel.video_published` | notification kind `video.published` | the recipient | `video:read` |
| `channel.subscriber_added` | `video.channel.subscribed` | `owner_id` | `video:read` |
| `note.created` | `notes.note.created` | `author_id` | `notes:read` |
| `note.mentioned` | notification kind `notes.mention` | the recipient | `notes:read` |
| `note.replied` | notification kind `notes.reply` | the recipient | `notes:read` |
| `comment.received` | `engagement.comment.published` | `subject_owner_id` | `engagement:read` |
| `reaction.received` | `engagement.reaction.changed` | `subject_owner_id` | `engagement:read` |
| `wallet.balance_changed` | `wallet.balance.changed` | `user_id` | `wallet:read` |
| `account.updated` | `identity.user.updated` | `user_id` | `identity:read` |
| `ping` | `WebhookService.PingWebhook` | — | — |

A delivery is `POST <url>` with the JSON body
`{ "id": <event id>, "type": <event type>, "time": <RFC 3339>, "data": … }`.
For a domain event `data` is its payload as documented by the emitting
domain; for a notification it is
`{ kind, actor_id, title: {ja, en}, body: {ja, en}, path, image_url, subject: {type, id}, data }`.
Notifications are taken from the request, before the notify domain applies
the recipient's mutes and category switches: a webhook hears every
notification addressed to its account.

Headers: `Tokuly-Event` (type), `Tokuly-Delivery` (delivery ID),
`User-Agent: Tokuly-Webhooks/1.0` and
`Tokuly-Signature: t=<unix seconds>,v1=<hex>[,v1=<hex>]`, where each `v1` is
`HMAC-SHA256(secret, "<t>.<body>")` (two while a rotated secret overlaps).
Receivers verify one `v1` and refuse timestamps more than a few minutes off;
`tokuly_dev::delivery::verify_signature` is the reference implementation.

Delivery rules:

- Any 2xx within `TOKULY_DEV_WEBHOOK_TIMEOUT_SECONDS` is success. Redirects
  are failures and never followed.
- A failed attempt is retried after `TOKULY_DEV_WEBHOOK_RETRY_SECONDS`, then
  four times longer each time (1 m, 4 m, 16 m, 64 m, 4.3 h, 17 h, 24 h), up to
  `TOKULY_DEV_WEBHOOK_MAX_ATTEMPTS` attempts.
- A webhook whose last `TOKULY_DEV_WEBHOOK_DISABLE_AFTER_FAILURES` attempts
  all failed, over at least `TOKULY_DEV_WEBHOOK_DISABLE_AFTER_HOURS`, is
  disabled: waiting deliveries are settled as failed and its owner notified.
- Only `https` URLs. The connection is made to an address verified to be
  public when it is resolved (no loopback, private, link-local, CGNAT,
  multicast or reserved ranges, in IPv4 or IPv6), without proxy.
- The fan-out keeps the set of subscribed event types in memory (10 s): an
  event of a type nobody subscribes to costs no query.
- All webhooks of one person (their bots' included) may send
  `TOKULY_DEV_WEBHOOK_RATE_PER_MINUTE` deliveries a minute to one host; events
  beyond that are dropped, not queued, so webhooks cannot be aimed at
  somebody's server as a flood.
- Webhooks of suspended accounts, of switched-off bots and of bots whose owner
  is suspended receive nothing.
- Attempts at one webhook are settled one at a time (row lock): it is disabled
  and its owner told exactly once, and an attempt that was in flight meanwhile
  is recorded but not retried.

## Bots

A bot is a row in identity's `users` table (no email, no password) plus a
row in `dev_bots` naming its owner. Identity has neither an API for such an
account nor a bot flag or owner link, so as an interim `tokuly_dev::bots`
writes the bot's `users` row itself: the insert, the profile update, and
`deletion_scheduled_at = now()`. Identity's own hourly purge then erases the
account and emits `identity.user.deleted`; this crate does not run the purge.

A bot token cannot hold wallet scopes, and write scopes only for `identity`,
`media`, `notify`, `search` and `feed` (`SCOPE_NOT_FOR_BOTS`; also applied at
verification). A bot therefore earns and spends nothing and can address
nobody, until blocks reach bots through the social domain.

What a bot sent stays with the people it was sent to, who can still report
it. So that such a report leads to a person, the `dev_bots` row of a deleted
bot is kept, marked deleted, for `TOKULY_DEV_BOT_RECORD_DAYS` counted from the
deletion (`Dev::bot_owner` answers for it meanwhile). A bot suspended by
moderation cannot be deleted by its owner (`BOT_SUSPENDED`) and keeps its
place in the owner's allowance; likewise an application disabled by moderation
cannot be deleted (`APP_DISABLED`).

## Notification kinds

All are system notices (no actor), inbox + push.

| Kind | Recipient | When |
|---|---|---|
| `dev.token_created` | the account | A personal access token was created for it. `data = { token_id, scopes }`. |
| `dev.app_authorized` | the user | They gave an application access for the first time. `data = { app_id, scopes }`. |
| `dev.app_disabled` | the application's owner | Moderation disabled the application. Subject: the application. |
| `dev.webhook_disabled` | the webhook's owner (a bot's owner) | Deliveries kept failing and the webhook was disabled. `data = { webhook_id }`. |

Security events written to the account's security log (through identity):
`API_TOKEN_CREATED`, `API_TOKEN_REVOKED`, `BOT_TOKEN_CREATED`,
`APP_AUTHORIZED`, `APP_ACCESS_REVOKED`, `APP_SECRET_ROTATED`.

## Background jobs and loops

| Job / loop | Payload | What |
|---|---|---|
| `dev.webhook.deliver` | `DeliverJob { delivery_id }` | One attempt at a delivery (concurrency 16). Idempotent: an attempt is claimed in the database first. |
| retry sweep, every 15 s | — | Queues `dev.webhook.deliver` for deliveries whose next attempt is due. |
| housekeeping, hourly | — | Deletes delivery log entries older than `TOKULY_DEV_WEBHOOK_LOG_DAYS`, OAuth token pairs that can no longer be used, and the ownership records of bots deleted more than `TOKULY_DEV_BOT_RECORD_DAYS` ago. |

## Rate limits

Besides the per-token request limit of the auth middleware (`RATE_LIMITED`):

| What | Limit |
|---|---|
| Creating tokens (personal and bot) | 20 / hour per person |
| Creating applications; editing them; rotating a client secret | 10 / hour; 60 / hour; 10 / hour per application |
| Consent screen and denial; approval | 120 / 5 min; 30 / 5 min per person |
| Typing device user codes (look-up, approve, deny) | 10 / 5 min per person |
| OAuth HTTP endpoints; of which starting device flows | 120 / min; 20 / min per address |
| Device polling | the flow's interval (`slow_down`, growing by 5 s) |
| Creating bots; editing a bot; changing a bot's handle; switching bots | 10 / day; 30 / hour; 5 / week per bot; 60 / hour |
| Creating webhooks; editing; rotating a secret | 30 / hour; 120 / hour; 10 / hour per webhook |
| Test pings; replays | 20 / min; 60 / hour per person |
| Usage statistics | 60 / min per person |
| API reference RPCs | 120 / min per address |

The limits on tokens, applications, bots and webhooks per account are
counted under an advisory lock per account, on the transaction that inserts.

## Analytics

`dev.api.request` (ClickHouse `events`): one per request made with an API
token, tracked by the auth middleware's hook after the response. Subject:
`("token", id)` for a personal token, `("app", id)` for an OAuth application,
`("bot", id)` for a bot; `user_id` the account acted as; `value` the
milliseconds taken; `props = { procedure, error }` (`error` is the reason,
empty on success). No owner. `UsageService.GetUsage` reads it.

## HTTP endpoints

`Dev::http_routes()`, to be merged into the API router inside the auth
middleware:

| Route | What |
|---|---|
| `POST /oauth/token` | RFC 6749 token endpoint: `authorization_code` (PKCE `S256` required), `refresh_token` (rotation, reuse revokes the family), `urn:ietf:params:oauth:grant-type:device_code`. Client authentication: `client_secret_basic`, `client_secret_post`, or none for public clients. |
| `POST /oauth/revoke` | RFC 7009. |
| `POST /oauth/device_authorization` | RFC 8628, only for applications registered with `device_flow` (`unauthorized_client` otherwise; codes issued earlier die when the flag is switched off). The user approves at `<public url>/device`. |
| `GET /.well-known/oauth-authorization-server` | RFC 8414 metadata. The issuer is `<public url><api path prefix>`. |
| `GET /reference/descriptor.binpb` | The `FileDescriptorSet` of the whole API. |

The authorization endpoint is the web app's page `/oauth/authorize`, driven
by `AuthorizationService`.

## Public Rust API

```rust
Dev::new(core, identity, DevConfig) -> anyhow::Result<Dev>      // registers the token verifier, the APP subject, the exporter
dev.register(router) -> connectrpc::Router
dev.http_routes() -> axum::Router
dev.spawn_workers()
dev.set_images(Arc<dyn tokuly_dev::ImageStore>)                  // bridge to tokuly_media::assets, for icons and avatars:
                                                                 //   image(uploader, asset_id) -> Image JSON   (reads; called outside transactions)
                                                                 //   hold(tx, asset_id, holder) / release(tx, holder)   (on the caller's transaction)
dev.bot_owner(bot_id) -> AppResult<Option<Uuid>>                 // for moderation; also for bots deleted within the retention
dev.set_app_rate_tier(app_id, "standard" | "elevated") -> AppResult<()>
dev.fan_out(&Event) -> AppResult<usize>                          // webhook consumer body
dev.on_account_event(&Event) -> AppResult<()>                    // account consumer body
dev.deliver(delivery_id) -> AppResult<Option<Delivery>>          // delivery job body
dev.queue_due_deliveries() -> AppResult<usize>
tokuly_dev::scopes::{catalogue, find, describe, validate}
tokuly_dev::secrets::{generate, kind_of, is, hint, SecretKind}
tokuly_dev::delivery::{signature_header, verify_signature, is_public_address, check_url}
tokuly_dev::webhooks::{EVENT_TYPES, event_type, EventType, Source}
```

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_DEV_MAX_TOKENS` | `50` | Live personal access tokens per account |
| `TOKULY_DEV_MAX_APPS` | `25` | Applications per account |
| `TOKULY_DEV_MAX_BOTS` | `5` | Bots per person |
| `TOKULY_DEV_MAX_BOT_TOKENS` | `10` | Live tokens per bot |
| `TOKULY_DEV_MAX_WEBHOOKS` | `20` | Webhooks per account (person or bot) |
| `TOKULY_DEV_RATE_STANDARD` | `600` | Requests per minute of a token (0 = unlimited) |
| `TOKULY_DEV_RATE_ELEVATED` | `3000` | Requests per minute in the `elevated` tier (`Dev::set_app_rate_tier`) |
| `TOKULY_DEV_ACCESS_TOKEN_SECONDS` | `3600` | Lifetime of an OAuth access token |
| `TOKULY_DEV_REFRESH_TOKEN_DAYS` | `90` | Lifetime of an OAuth refresh token (renewed on use) |
| `TOKULY_DEV_AUTH_CODE_SECONDS` | `60` | Lifetime of an authorization code |
| `TOKULY_DEV_DEVICE_CODE_SECONDS` | `900` | Time a device has to be approved |
| `TOKULY_DEV_DEVICE_POLL_SECONDS` | `5` | Minimum pause between a device's polls |
| `TOKULY_DEV_SECRET_OVERLAP_HOURS` | `24` | How long a rotated client or webhook secret keeps working |
| `TOKULY_DEV_WEBHOOK_TIMEOUT_SECONDS` | `10` | Time an endpoint has to answer |
| `TOKULY_DEV_WEBHOOK_MAX_ATTEMPTS` | `8` | Attempts per delivery |
| `TOKULY_DEV_WEBHOOK_RETRY_SECONDS` | `60` | Pause before the first retry (fourfold after) |
| `TOKULY_DEV_WEBHOOK_DISABLE_AFTER_FAILURES` | `20` | Failed attempts in a row before a webhook is disabled... |
| `TOKULY_DEV_WEBHOOK_DISABLE_AFTER_HOURS` | `48` | ...once it has been failing this long |
| `TOKULY_DEV_WEBHOOK_RATE_PER_MINUTE` | `120` | Deliveries per minute one person's webhooks may send to one host (0 = unlimited) |
| `TOKULY_DEV_WEBHOOK_LOG_DAYS` | `30` | Retention of the delivery log |
| `TOKULY_DEV_BOT_RECORD_DAYS` | `180` | How long the record of who owned a deleted bot is kept |
| `TOKULY_DEV_WEBHOOK_ALLOW_PRIVATE` | `false` | Development/tests: allow `http://` and private addresses. Startup fails if set in production |

Requests made with tokens are also subject to `TOKULY_RATE_LIMITS`
(switching it off switches the per-token limit off).
