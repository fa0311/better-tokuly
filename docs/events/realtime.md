# realtime

The gateway (`backend/crates/realtime`) owns no tables and emits no domain
events. It connects clients to the bus: frames published by the domains go
down, what clients say goes up. Protocol: `proto/tokuly/realtime/v1/realtime.proto`;
contracts towards the domains: `tokuly_core::presence`, `tokuly_core::realtime`.

## The envelope

The protocol knows the gateway's own messages (`Welcome`, `Pong`, `Ack`,
`Error`, `PresenceEvent`, `TopicStats`, `Unsubscribed`) and nothing of the domains. What a
domain pushes travels as `ServerFrame { topic, event: Event { type, payload } }`:
`type` is the full name of the domain's event message
(`tokuly.chat.v1.ChatEvent`) and `payload` that message, encoded. The types
in use are listed at `Event` in the proto file.

- A domain publishes with `core.rt.notify_user(user_id, &event)` and its
  siblings (`backend/README.md`, "Talking to other domains") and never
  builds a frame; tests read one back with `frame.event::<ChatEvent>()`.
- `@tokuly/realtime` decodes `Event` by a table from type name to message
  (`packages/realtime/src/events.ts`) and hands listeners
  `frame.body.case === "chat"` with the `ChatEvent` as `value`. A domain's
  schemas load with its first event; the frames behind it wait, in order.
- Clients skip events whose type they do not know, so a server may start
  sending a new type before every client understands it.
- Upwards, `ClientFrame.location` carries an encoded
  `tokuly.geo.v1.LocationSample` as bytes — on the wire the same as a field
  of that type.

## Running it

```rust
let gateway = Gateway::new(core.clone(), realtime_config)?;
let router = gateway.register(router);          // every API process: RealtimeService.GetEndpoints
tokio::spawn(async move { gateway.run().await }); // gateway processes only; returns after core.shutdown
```

`Gateway::run` binds the WebTransport (UDP) and WebSocket (TCP) listeners.
Several gateway processes may run side by side; clients may land on any.

## Bus subjects (core NATS, namespaced by `TOKULY_NATS_PREFIX`)

Consumed — one subscription per distinct user/topic with a connection on the
gateway, created by the first connection and dropped with the last:

| Subject | Payload | Forwarded to |
|---|---|---|
| `rt.u.<user>` | encoded `ServerFrame` | every connection of the user |
| `rt.t.<topic>` | encoded `ServerFrame`; optional header `Tokuly-Except` | every connection subscribed to the topic, except those signed in as a user the header names |
| `rt.u.<user>`, `rt.t.<topic>` with header `Tokuly-Control` | empty | not forwarded: a revocation for the sessions (see "Ending subscriptions") |

Published:

| Subject | Payload | When |
|---|---|---|
| `sys.presence` | `PresenceChange` | a user's first foreground connection on any gateway appears / the last one goes (also when a dead gateway's users are reaped) |
| `loc.sample.<user>` (stream `LOCATIONS`) | `LocationReport` | a signed-in client sent `ClientFrame.location` that passed the sanity checks |
| `sys.typing` | `TypingSignal` | `ClientFrame.typing` |
| `sys.conversation_presence` | `ConversationPresenceSignal` | `ClientFrame.conversation_presence` |
| `rt.t.<topic>` | `ServerFrame { topic_stats }` | the topic's audience changed (at most every `TOKULY_RT_TOPIC_STATS_INTERVAL` per topic and gateway) |

The gateway does not check that a user belongs to the conversation named in
a typing/presence signal; the chat domain must.

## Frame delivery

- Frames are forwarded verbatim. The gateway only peeks at the event's
  type and first tag: a `GeoEvent` with `location` (a friend's position) is
  *droppable* — sent as a datagram
  over WebTransport when it fits, and the oldest queued one is discarded
  when a client reads too slowly. Everything else is *reliable*: a client
  whose backlog exceeds `TOKULY_RT_QUEUE_FRAMES` / `TOKULY_RT_QUEUE_BYTES`
  is disconnected and resynchronizes on reconnect.
- A topic frame may leave some users out (`Realtime::to_topic_except`, at
  most 1000, `tokuly_core::realtime::MAX_EXCEPT`): the gateway skips every
  connection signed in as one of them. An anonymous connection is never
  left out.
- `Welcome` is sent after the user's subscription is in place, and after
  the topics named in `Hello` are (the refused ones are announced with
  `Unsubscribed` before it): a client that fetches a snapshot after
  `Welcome` misses no event of the user or of those topics.
- A `Subscribe` sent with an ID is answered once it is in effect: `Ack`
  (a snapshot fetched after it misses no event of its topics), or `Error`
  when it was refused as a whole (`TOPIC_FORBIDDEN`; `TOPIC_CHECK_FAILED` /
  `BUS_UNAVAILABLE` when it could not be decided now). In `@tokuly/realtime`,
  `subscribe(topic).ready` settles with it, and `onSubscribed` runs each
  time the topic is in effect again after a reconnect: that is where a
  feature reads its snapshot.
- Delivery is at most once. Anything that must not be lost has to be
  fetchable from the owning domain.

## Topics

`<kind>:<id>[:<id>...]` — lowercase kind, segments of letters, digits, `-`,
`_`; at most 128 bytes. Kinds are public unless the owning domain registers a
`tokuly_core::realtime::TopicGuard` with `core.topic_guards.register(kind, guard)`
when it is constructed (so it is in place in every process, including the
gateway's). The gateway asks `core.topic_guards.may_subscribe(subscriber, topic)`
when a connection subscribes — in `Subscribe` and for topics named in
`Hello` — with the subscriber's user ID (`None` = anonymous), again when a
domain revokes the subscription, and periodically while it lasts (below).
On `Subscribe`, `Ok(false)` is answered with `TOPIC_FORBIDDEN`, an error
with `TOPIC_CHECK_FAILED`; either way nothing of that request is
subscribed. In `Hello` a refused topic does not refuse the Hello: the
connection gets `Unsubscribed { topic, reason: TOPIC_FORBIDDEN }` for that
one (before or after `Welcome`; clients handle either order).

## Ending subscriptions

When access ends after someone subscribed, the owning domain tells the
gateways once its change is committed (`tokuly_core::realtime`):

| Call | Bus message | Effect on every gateway |
|---|---|---|
| `rt.revoke(user, topic)` | `rt.u.<user>`, `Tokuly-Control: recheck`, `Tokuly-Topic: <topic>` | each connection of the user that follows the topic asks the guard again and drops it if refused |
| `rt.revoke_prefix(user, prefix)` | the same with `recheck-prefix` | as above, for every topic equal to the prefix or below it (`party:<id>` covers `party:<id>:<key>`; `live` covers every `live:` topic; whole segments only) |
| `rt.recheck_topic(topic)` | `rt.t.<topic>`, `Tokuly-Control: recheck` | every subscriber, anonymous ones included, is asked about again — within the gateway's re-check budget (below), so a big audience is gone through over seconds rather than with every guard query at once. Call it only when an audience narrows |
| `rt.close_topic(topic)` | `rt.t.<topic>`, `Tokuly-Control: close` | every subscriber is dropped without asking (the room was deleted, the share or stream is over) |

A dropped subscription is announced to its connection with
`ServerFrame { topic, unsubscribed: Unsubscribed { topic, reason } }`,
`reason` `TOPIC_FORBIDDEN` (the guard refused) or `TOPIC_CLOSED`. Nothing
more of the topic reaches the connection; a new `Subscribe` runs the guard
again. Clients do not subscribe again by themselves.

- **The guard decides.** A revocation is "ask again", so a domain calls it
  on any change that *might* end access, and twice is the same as once. A
  guard that fails on the re-check keeps the subscription and is asked again
  10 s later: an outage is not a revocation.
- **No race with subscribing.** A revocation that arrives while a
  `Subscribe` is being checked is applied to the subscription that check
  made.
- **Safety net.** Revocations are best-effort like every push. Each
  connection also re-checks every subscription older than
  `TOKULY_RT_RECHECK_INTERVAL` (5 min) — which bounds how long a lost
  revocation can leave someone listening — at most
  `TOKULY_RT_RECHECK_BATCH` (8) at once, and the whole gateway at most
  `TOKULY_RT_RECHECK_RATE` (200) per second (`recheck_topic` shares that
  budget); beyond it the cycle stretches rather than the database load
  growing.
- Counted in the gateway stats as `topic_rechecks` and `topics_revoked`.

## Errors sent to clients (`Error.code` / `Error.message`)

| Code | Message | Fatal | Meaning |
|---|---|---|---|
| `unauthenticated` | `TOKEN_INVALID`, `SESSION_REVOKED` | yes | sign in again |
| `unauthenticated` | `TOKEN_EXPIRED` | on `Hello`: yes; pushed (`reply_to` 0): no, then yes after the grace period | refresh the token, send `Hello` again on the same connection (or reconnect) |
| `unauthenticated` | `SIGN_IN_REQUIRED` | no | frame needs a signed-in connection |
| `permission_denied` | `USER_MISMATCH` | yes | `Hello` for a different user than the connection's |
| `permission_denied` | `TOPIC_FORBIDDEN` | no | a `TopicGuard` refused |
| `failed_precondition` | `HELLO_REQUIRED` | no | only `Hello` and `Ping` are accepted before `Welcome` |
| `failed_precondition` | `HELLO_TIMEOUT`, `IDLE_TIMEOUT` | yes | no `Hello` / no frame in time |
| `invalid_argument` | `FRAME_INVALID`, `TOPIC_INVALID`, `LOCATION_INVALID`, `CONVERSATION_ID_INVALID`, `DATAGRAM_NOT_ALLOWED` | no | |
| `invalid_argument` | `FRAME_TOO_LARGE`, `BINARY_FRAMES_ONLY` | yes | |
| `resource_exhausted` | `TOO_MANY_TOPICS`, `RATE_LIMITED` | no | |
| `unavailable` | `BUS_UNAVAILABLE` | on `Hello`: yes; on `Subscribe`: no | retry / reconnect with backoff |
| `unavailable` | `TOPIC_CHECK_FAILED` | no | the topic guard failed; retry |
| `unavailable` | `SHUTTING_DOWN` | yes | reconnect (another instance takes over) |

Frames sent with `id = 0` get no `Ack`/`Error`. Close codes (WebSocket close
code, WebTransport session/connection code): 4000 closed, 4001 shutting down,
4002 after a fatal `Error`, 4003 idle, 4004 slow consumer.

Connection attempts can be refused before any frame is exchanged:

| WebSocket upgrade | WebTransport session | Reason |
|---|---|---|
| `403` | `403` | `Origin` not allowed |
| `404` | `404` | wrong path |
| `503` | `429` | the gateway is at `TOKULY_RT_MAX_CONNECTIONS` |
| `429` | `429`, or the QUIC connection is refused outright | the client address is at `TOKULY_RT_MAX_CONNECTIONS_PER_IP` |

Clients should back off and retry; a per-address refusal usually means
connections are being leaked (reconnecting without closing).

## Valkey keys

| Key | Type | Purpose |
|---|---|---|
| `rt:online:<user>`, `rt:seen:<user>` | string | owned by `tokuly_core::presence` (`mark_online` / `mark_offline`) |
| `rt:topic:<topic>` | hash instance → report | owned by `tokuly_core::presence`; the gateway only calls `set_topic_audience` / `topic_audience` and rewrites its reports every `TOKULY_RT_AUDIENCE_REFRESH` (they go stale after 90 s) |
| `rt:fg:u:<user>` | zset instance → lease expiry (ms) | which gateways hold a foreground connection of the user |
| `rt:fg:leases` | zset user → latest lease expiry (ms) | index for reaping the leases of dead gateways |
| `rt:wt:certs` | zset hex SHA-256 → valid-until (ms) | self-signed certificate hashes returned by `GetEndpoints` |

## Configuration

Durations take a unit (`500ms`, `25s`, `2m`, `1h`, `13d`); a bare number is seconds.

| Variable | Default | |
|---|---|---|
| `TOKULY_RT_WT_LISTEN` | `0.0.0.0:4433` | WebTransport (UDP). Empty disables it |
| `TOKULY_RT_WS_LISTEN` | `0.0.0.0:8082` | WebSocket `/rt/ws` and `/healthz` (TCP) |
| `TOKULY_RT_TLS_CERT`, `TOKULY_RT_TLS_KEY` | unset | PEM files of a publicly trusted certificate (re-read every 5 min). Unset: self-signed + pinned hashes |
| `TOKULY_RT_CERT_VALIDITY` | `13d` | lifetime of a self-signed certificate (max 13d); next one announced at 60 %, switched to at 75 % |
| `TOKULY_RT_PUBLIC_WT_URL` | `https://<public host>:<WT port>/rt` | returned by `GetEndpoints` |
| `TOKULY_RT_PUBLIC_WS_URL` | production: `wss://<public origin>/rt/ws`; development: `ws://<public host>:<WS port>/rt/ws` | returned by `GetEndpoints` |
| `TOKULY_RT_ALLOWED_ORIGINS` | empty | extra browser origins (comma separated) besides `TOKULY_PUBLIC_URL`, the public gateway URLs and, in development, localhost |
| `TOKULY_RT_MAX_CONNECTIONS` | `100000` | per gateway process |
| `TOKULY_RT_MAX_CONNECTIONS_PER_IP` | `1024` | per client address (IPv4 address or IPv6 /64), both transports together; `0` disables. Sized for a campus or mobile carrier behind one address |
| `TOKULY_RT_TRUSTED_PROXIES` | loopback and private ranges (`127.0.0.0/8,::1,10.0.0.0/8,172.16.0.0/12,192.168.0.0/16,fc00::/7`) | peers whose `X-Forwarded-For` names the WebSocket client (addresses or CIDRs, comma separated); from anyone else the header is ignored |
| `TOKULY_RT_HELLO_TIMEOUT` | `10s` | |
| `TOKULY_RT_PING_INTERVAL` | `25s` | told to clients in `Welcome` |
| `TOKULY_RT_IDLE_TIMEOUT` | `60s` | no frame for this long closes the connection |
| `TOKULY_RT_AUTH_GRACE` | `60s` | time to re-`Hello` after token expiry |
| `TOKULY_RT_REVOCATION_CHECK` | `10s` | per-connection session revocation re-check |
| `TOKULY_RT_SHUTDOWN_GRACE` | `5s` | |
| `TOKULY_RT_MAX_FRAME_BYTES` | `65536` | inbound |
| `TOKULY_RT_MAX_TOPICS` | `64` | per connection |
| `TOKULY_RT_QUEUE_FRAMES`, `TOKULY_RT_QUEUE_BYTES` | `1024`, `4194304` | reliable backlog per connection |
| `TOKULY_RT_QUEUE_DROPPABLE_FRAMES` | `128` | queued friend positions per connection |
| `TOKULY_RT_WRITE_TIMEOUT` | `15s` | |
| `TOKULY_RT_LOCATION_BURST`, `TOKULY_RT_LOCATION_RATE` | `60`, `2` | location samples per connection (burst, then per second) |
| `TOKULY_RT_MOVING_INTERVAL`, `TOKULY_RT_STATIONARY_INTERVAL` | `5s`, `60s` | cadence hints in `Welcome` |
| `TOKULY_RT_PRESENCE_HEARTBEAT`, `TOKULY_RT_PRESENCE_LEASE` | `25s`, `75s` | online lease renewal / lifetime |
| `TOKULY_RT_AUDIENCE_REFRESH` | `30s` | rewrite of topic audience reports; at most `45s` (reports expire after 90 s) |
| `TOKULY_RT_TOPIC_STATS_INTERVAL` | `2s` | throttle of `TopicStats` pushes |
| `TOKULY_RT_RECHECK_INTERVAL` | `5m` | age at which a topic subscription is checked against its guard again |
| `TOKULY_RT_RECHECK_BATCH` | `8` | most periodic re-checks of one connection at once |
| `TOKULY_RT_RECHECK_RATE` | `200` | most periodic re-checks per second on one gateway |
| `TOKULY_RT_STATS_INTERVAL` | `60s` | stats log line |

## Deployment notes

- **WebTransport** needs the gateway's UDP port reachable directly (it is
  HTTP/3; an HTTP/1–2 reverse proxy cannot forward it). With the default
  self-signed certificate, browsers connect with
  `new WebTransport(url, { serverCertificateHashes: [{ algorithm: "sha-256", value }] })`
  using every hash from `GetEndpoints`; fetch them right before connecting
  and again after a failed attempt.
- **WebSocket** goes through the edge proxy: route `/rt/ws` on the public
  origin to the gateway's TCP port, pass the upgrade (`Upgrade`/`Connection`
  headers, HTTP/1.1 to the upstream), keep `Origin` and `Host` intact, set
  `X-Forwarded-For`, disable buffering and use read/send timeouts above
  `TOKULY_RT_IDLE_TIMEOUT`.
- **Client addresses** (for the per-address limit): WebTransport uses the
  QUIC peer address. WebSocket uses the TCP peer, unless that peer is in
  `TOKULY_RT_TRUSTED_PROXIES`; then `X-Forwarded-For` is read from the right
  and the first address that is not a trusted proxy is the client (entries
  further left are client-supplied and ignored). The proxy must therefore
  append or set the address it saw. With a CDN in front of the edge proxy,
  add the CDN's ranges, or every client behind one CDN node shares a limit.
  If the gateway's WebSocket port is reachable without a proxy from a
  private network, set the variable to empty.
