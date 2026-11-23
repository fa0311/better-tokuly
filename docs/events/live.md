# live — events, jobs and configuration

Crate `tokuly-live`, migration `0900_live.sql`, API `tokuly.live.v1`
(`LiveStreamService`, `LiveChatService`, `LivePollService`,
`LiveGiftService`, `ClipService`, `OverlayService`, `LiveAnalyticsService`,
realtime `LiveEvent`). Depends on `identity`, `social`, `media`, `stamp`,
`wallet` and `video`. The media server is MediaMTX 1.21
(`infra/mediamtx/mediamtx.yml`).

## API for other domains

```rust
// Clips (shorts: "make a short from a clip").
tokuly_live::clips::find(core, &Ref) -> AppResult<Option<ClipRow>>          // UUID or short ID, any state
tokuly_live::clips::source_of(core, &viewer, &clip) -> AppResult<Option<Source>>   // .watchable: may this viewer see the clip
// ClipRow: id, short_id, creator_id, channel_id, owner_id, stream_id, video_id, title, start_seconds,
//          duration_seconds, status (READY = 2), asset_id, playback, thumbnail

// Streams.
tokuly_live::streams::find(core, &Ref) -> AppResult<Option<StreamRow>>      // deleted streams are not found
tokuly_live::streams::current_of_channel(core, channel_id) -> AppResult<Option<StreamRow>>   // on air (or encoder connected)
tokuly_live::access::can_watch(&viewer, &stream.facts()) -> bool
```

A READY clip's `asset_id` is a READY video asset (`ASSET_PURPOSE_SHORT`)
owned by the **owner of the channel** it was cut from (`owner_id`, not the
person who cut it) and held by `live:clip:<clip_id>`; a domain that reuses it
attaches its own holder.

**Wiring the server does** (`crates/server/src/app.rs`, `compose.yaml`,
`infra/caddy/Caddyfile`; all of it is in place):

- construct `Live::new(core, identity, cfg, wallet)` in every process (it
  registers the guards of the `live:` and `overlay:` realtime topics, which
  the gateway needs), `register` it, `spawn_workers` in worker processes, add
  `LiveConfig` to `DomainConfigs`;
- mount `live.http_routes()` at `/hooks/live` on the API listener, served
  with `into_make_service_with_connect_info::<SocketAddr>()` (it is today):
  the hook counts wrong secrets per peer address. **This route is for the
  media server only** (see "Who may call the hook" below for what the edge
  has to block);
- add MediaMTX to `compose.yaml` with `infra/mediamtx/mediamtx.yml`, a volume
  for `/recordings` shared with the worker (`TOKULY_LIVE_RECORDINGS_DIR`),
  `MTX_AUTHHTTPADDRESS=http://mediamtx:<TOKULY_LIVE_HOOK_SECRET>@api:8081/hooks/live/auth`
  and `MTX_WEBRTCADDITIONALHOSTS=<public host>`; publish 1935 (RTMP),
  8890/udp (SRT), 8888 (HLS), 8889 + 8189/udp (WebRTC) and 9996 (DVR), not
  9997 (control API). The API and the worker get `TOKULY_LIVE_HOOK_SECRET`
  and `TOKULY_MEDIAMTX_API_SECRET` (two different values). The worker needs
  `ffmpeg` and `ffprobe`.

## The media server

Every stream is the MediaMTX path `live/<short_id>`. MediaMTX asks
`POST /hooks/live/auth` (`authMethod: http`) before anyone publishes, reads,
uses the playback server or its control API:

| `action` | Allowed when |
|---|---|
| `publish` | the path is a stream that is not over, deleted or suspended, and the credential (`token`, or `password` for SRT) is its stream key |
| `read` | the stream is on air and PUBLIC / NO_FEED / UNLISTED; otherwise the request carries a valid playback token (`?t=`, or as bearer token) |
| `playback` | as `read`; the stream is on air with DVR on, and `start` lies after `started_at` and within `TOKULY_LIVE_DVR_WINDOW_SECONDS` |
| `api`, `metrics`, `pprof` | user `tokuly` with `TOKULY_MEDIAMTX_API_SECRET` as password (the worker) |

MediaMTX records everything an encoder sends, also the owner's preview before
the stream goes on air. Rewinding never reaches that: a `start` before
`started_at` is refused, a stream that is not on air (not yet, or over) has no
DVR at all, and clips and the archive are cut from `started_at` on.

### Who may call the hook

Only MediaMTX. It proves it with `TOKULY_LIVE_HOOK_SECRET` in the
`Authorization` header: the credentials in `authHTTPAddress`
(`http://mediamtx:<secret>@api:8081/hooks/live/auth`) are sent as HTTP Basic,
and `Bearer <secret>` is accepted as well. The secret is **not** read from the
URL any more (`?secret=` is refused), so it does not end up in access logs.

- A peer address that presents a wrong secret 10 times within a minute gets
  `429` for the rest of that minute, whatever it sends
  (`live:hook-failures:<address>`). A wrong secret is `403`.
- The hook secret and the control API secret are two secrets
  (`LiveConfig::validate` refuses equal ones): whoever learns the one MediaMTX
  holds cannot read or kick paths with it, and the hook does not accept the
  API secret.

**What the edge must block.** The hook answers on the API listener at
`/hooks/live/auth`. The Caddyfile forwards `/api/*` to that listener with the
prefix stripped, so the edge has to refuse, for every method:

- `/api/hooks/*` (which would arrive as `/hooks/*`), and `/hooks/*` itself if
  the edge ever forwards unprefixed paths to the API;

and the stack must not publish the API listener's port (8081) or MediaMTX's
control API (9997) to the outside. Behind the edge every request would also
share the edge's address, so the per-address lockout would be one bucket for
everyone: the block is what protects the hook, the lockout is the second line.

How the stack does it: `infra/caddy/Caddyfile` answers `404` for `/api/hooks`,
`/api/hooks/*`, `/hooks` and `/hooks/*`; `compose.yaml` publishes neither
8081 nor 9997; and `tools/ci/smoke.sh` asks for all of them from outside.

**In the Compose stack.** `tools/init-env.sh` generates the two secrets
(`TOKULY_LIVE_HOOK_SECRET`, `TOKULY_MEDIAMTX_API_SECRET`); run again on a
deployment that has a `.env` already, it adds just these and changes nothing
else. MediaMTX runs as
the backend's user (10001) and records into the `recordings` volume, which
the worker mounts at `/recordings`; the backend image owns that directory,
and the worker starts first, so a fresh volume belongs to that user and the
worker can delete what was recorded. `TOKULY_LIVE_PUBLIC_HOST` and the
`TOKULY_LIVE_*_URL` settings in `.env` name the machine as encoders and
players reach it.

**In development** MediaMTX is started on request only, because
`tests/mediamtx.rs` starts its own on the same ports:
`TOKULY_DEV_MEDIAMTX=1 tools/dev/infra.sh up` (or the `live` profile of
`compose.dev.yaml`), then `eval "$(tools/dev/infra.sh env)"`, which prints
the `TOKULY_LIVE_*` settings while that media server runs.

### Playback tokens

Stream keys are stored encrypted and hashed. A playback token is
`<expiry>.<viewer>.<keyed hash of stream, viewer and expiry>`: it opens one
stream, for 15 minutes, and names the account it was made for. `GetStream`
hands it out to whoever may watch a restricted stream (and to the owner for
the preview before going on air); every `ReportWatching` answer carries fresh
playback URLs, so a player that keeps reporting always holds a valid token.
A token is only needed to **open** a media session (an HLS session keeps its
query, a WebRTC session stays connected), so the short life does not interrupt
anyone who is watching.

**A token is worth what its viewer may do right now.** Every time MediaMTX
asks, the hook checks the token's viewer against the stream again
(`can_watch`; before the stream is on air only its owner): unfriended,
blocked or shut out by a change of visibility, a token that is still valid
opens nothing. A "yes" is cached for 10 s per stream and viewer
(`live:may-watch:…`); the cache of an owner's streams is dropped at once when
their visibility changes (`UpdateStream`) and on `social.friend.removed` /
`social.user.blocked` (`live:access-epoch:<owner>`). A "no" is never cached.

**Open sessions are cut on the media server.** Observed with MediaMTX 1.21.4
(`tests/mediamtx.rs` runs this against the real binary):

- MediaMTX asks the hook **once per media session**. For HLS that is the
  request for the multivariant playlist; the media playlists and segments
  are then served on the session's own ID (`?session=…`) without asking
  again. So checking the token on every hook request is not enough to stop
  someone who is already watching.
- Each HLS viewer is listed as a reader of type `hlsSession` on the path, and
  its ID **is** the `id` of the auth request that opened it.
- `POST /v3/hls/sessions/kick/<id>` ends such a session: its next playlist or
  segment request is answered 401 and the reader disappears from the path.

The hook therefore remembers every session it lets in with its viewer
(`live:sessions:<stream>`; anonymous sessions as nobody's), and
`Live::enforce_access(&stream, cut_unknown)` cuts the readers of a restricted
stream:

| Reader | Regular pass (`cut_unknown = false`) | After a change (`true`) |
|---|---|---|
| remembered, viewer may watch | stays | stays |
| remembered, viewer may not (any more) | cut | cut |
| remembered as nobody's (opened while the stream was public) | cut | cut |
| not remembered (bookkeeping lost) | stays | cut; its player reconnects through the hook |

It runs with `cut_unknown` when `UpdateStream` changes the visibility and on
`social.friend.removed` / `social.user.blocked` (the restricted live streams
of both people), and without it every tenth reconciler run. Lost bookkeeping
therefore never throws legitimate viewers out again and again: at worst they
reconnect once, right after a change, and whoever may not watch is refused by
the hook when they come back.

Limits: a token cannot be bound to an IP address or a device, so one that is
passed on works for someone else for up to 15 minutes, as that viewer, and is
cut and refused together with them. WebRTC, RTMP and SRT readers are handled
by the same code (asked once, kicked by ID) but only the HLS path is proven
against the real server. The DVR (playback server) has no sessions: every
request asks the hook, so there a lost access ends at once.

Lifecycle comes from the control API, not from hooks: the reconciler reads
`/v3/paths/list` every `TOKULY_LIVE_RECONCILE_INTERVAL_SECONDS` and compares.
A path that came online marks the encoder connected (and puts an `auto_start`
stream on air); a path that went away starts the reconnect grace of a stream
on air, which ends when the encoder stays away; a publisher on a path nobody
may publish to is kicked. Since states are compared, a restart of MediaMTX or
of the worker loses nothing. When MediaMTX cannot be reached for 5 minutes
its encoders are taken for gone.

Viewer count = the largest of: readers of the path (each HLS session,
WebRTC session, RTMP/SRT reader), connections subscribed to `live:<id>`
(`presence::topic_audience`), and players reporting with `ReportWatching`.
One person is usually in all three, so they are not added up.

Guests are whatever `client_id` a request names, so they are capped: at most
5 anonymous viewers per address (IPv4 address, IPv6 /64) count in one
stream's audience at a time (`live:guests:<stream>:<address group>`); further
ones are answered normally and not counted. And the directories ("live now",
by game, by category) do **not** rank by this number: they order by
`ranked_viewer_count`, the signed-in accounts among the heartbeats, which
costs a real account per viewer to move. Known limit: the *displayed*
`viewer_count` of a public stream can still be raised by opening many media
sessions or realtime connections (from many addresses); it no longer buys a
place in any list.

Recording: MediaMTX writes fMP4 segments to
`<recordings dir>/live/<short_id>/<unix seconds>-<microseconds>.mp4`
(`recordPath: /recordings/%path/%s-%f`); its playback server serves the DVR.
MediaMTX never deletes them (`recordDeleteAfter: 0s`); the worker does.

## Events emitted

Payload structs live in `tokuly_live::events`. All are written in the
transaction of the change they describe.

| Topic | Payload | When |
|---|---|---|
| `live.stream.started` | `StreamEvent` | A stream went on air. Actor: the owner. |
| `live.stream.ended` | `StreamEvent` | A stream is over (`end_reason`: `EndReason` number; `went_live` false = cancelled before it started). Actor: the owner, or nobody. |
| `live.stream.deleted` | `StreamEvent` | A stream is gone (always after `ended`). |
| `live.stream.archived` | `{ stream_id, video_id, channel_id, owner_id }` | The recording became a video. |
| `live.gift.sent` | `{ gift_id, stream_id?, channel_id, sender_id, recipient_id, coins, net_coins, tier }` | A gift settled (once per gift). Actor: the sender. |
| `live.clip.ready` | `ClipEvent` | A clip finished processing. Actor: its creator. |
| `live.clip.deleted` | `ClipEvent` | A clip is gone. |

`StreamEvent` is `{ stream_id, short_id, channel_id, owner_id, visibility, end_reason, went_live }`;
`ClipEvent` is `{ clip_id, short_id, creator_id, channel_id, stream_id?, video_id?, asset_id? }`.

## Events consumed

Durable consumer `live`. Every handler gives the same result when an event
arrives twice, late, or after what it names is gone.

| Topic | Reaction |
|---|---|
| `live.stream.started` (own) | Queues the announcement (`live.announce`), once per stream. |
| `live.stream.ended` (own) | Queues `live.archive` when the stream is to be archived. |
| `media.asset.ready` / `.failed` | The clip cut into that asset becomes READY (playback and thumbnail copied in, `live.clip.ready`, search document) or FAILED. |
| `media.asset.deleted` | A stream thumbnail is cleared; a clip with that upload is deleted; an archive whose upload vanished before its video existed is FAILED. |
| `video.channel.subscribed` | On the channel's stream on air: counted in `new_subscribers` and announced by the alert boxes once per person and stream; subscriber goals are pushed. |
| `video.channel.deleted` | The channel's streams are ended and deleted, its clips, overlays, moderators, restrictions and banned words removed. |
| `video.video.deleted` | Clips of the video are deleted; a stream it was the archive of has no archive. |
| `video.video.published` / `.unpublished` | The search documents of the video's clips are rewritten. |
| `admin.content.removed` | `LIVE_STREAM`: ended (no archive) and deleted. `CLIP`: deleted, its upload removed by force. `LIVE_CHAT_MESSAGE`: removed from the chat. |
| `identity.user.deleted` | The user's streams are erased (not kept), the clips they cut or that were cut from their channels deleted, their chat messages, votes, reminders, moderator seats, restrictions, overlays and banned words removed. Gifts stay as bookkeeping with the user's ID nulled; unpaid ones are dropped. |
| `identity.user.banned` / `.unbanned` | Only a signal to look: `live_streams.suspended` follows whether the account is banned *now* (the two can arrive in any order). A stream on air ends; streams leave or return to search. |
| `social.user.blocked` | Moderator appointments and reminders between the two are removed, and the blocked person's media sessions on the blocker's restricted live streams are cut (`enforce_access`). (A blocked user does not see the blocker's streams at all, immediately.) |
| `social.friend.removed` | The media sessions either of the two holds on the other's restricted (friends-only) live streams are cut. |

## Deleting, and what stays reportable

- A chat message that is deleted (by its author, a moderator, a ban with
  `delete_messages`) leaves the chat but its row stays; the
  `LIVE_CHAT_MESSAGE` resolver still resolves it, so it can be reported.
- A deleted stream (owner, channel deletion, takedown) is gone for every RPC
  at once, with its recording, reminders and thumbnail. The row and its chat
  are kept for `TOKULY_LIVE_DELETED_RETENTION_DAYS` **counted from the
  deletion**, during which the stream and its messages still resolve as
  subjects (interactions disabled), and are then erased by the sweep.
- Account erasure erases at once.

## Gifts and money

`SendGift` writes the gift down first (PENDING, with tier, message and
highlight time as decided then), has the wallet move the coins under the
ledger key `live.gift:<gift id>` (reason `live.gift`, memo
`{ gift_id, stream_id, channel_id }`), and settles it in one transaction
(`UPDATE … WHERE status = 1`) together with the chat line, the stream's
counters, `live.gift.sent` and the notification. A PENDING gift left behind
by an interrupted request is finished by the sender's next `SendGift` /
`ListSentGifts` and by the sweep; one the wallet refuses is removed. The row
is unique per sender and idempotency key.

A gift whose stream was deleted while it was PENDING is not kept. The settling
transaction locks the stream row and the gift row and decides once: SETTLED
when the stream is still there, otherwise RETURNING (status 3). A RETURNING
gift is refunded (`ledger::refund`, reason `live.gift.returned`), the row is
removed and the sender gets `failed_precondition` `GIFT_RETURNED`; the
streamer earns nothing from it. Because the decision is made under both
locks, any number of settlers racing with a deletion leave the gift either
settled or returned, never both; an interrupted return is finished by the
same entry points as an interrupted payment. Only when the refund is no
longer possible (`REFUND_IMPOSSIBLE`) does the gift stand, as bookkeeping
without a chat line.

The transfer hides its recipient
(`Wallet::transfer_with_fee_hiding_recipient`): a channel does not tell which
account runs it, so the sender's wallet history, live balance update, data
export and a later refund show the entry with `counterpart_hidden` and no
counterpart; the memo names the channel and the stream. The streamer sees the
sender (the chat showed the gift to everyone).

Only SETTLED gifts count anywhere: totals, top supporters, goal widgets.
Gift messages obey the chat's rules like any message: banned words, a ban or
timeout, and `LINK_POLICY_BLOCK` (`LINKS_NOT_ALLOWED`).

The wallet also sends its own `wallet.received` notification for the
transfer; `live.gift` is additional and only sent while the streamer's
dashboard is closed.

## Media assets

| Holder | Asset |
|---|---|
| `live:stream:<stream_id>` | the stream's thumbnail (`ASSET_PURPOSE_THUMBNAIL`) |
| `live:clip:<clip_id>` | the clip's upload (`ASSET_PURPOSE_SHORT`, owned by the owner of the channel it was cut from, created with `create_asset_from_file`) |

Clips and what they may expose:

- A clip can only be cut from a source that is PUBLIC, NO_FEED or UNLISTED
  (`CLIP_SOURCE_NOT_PUBLIC` otherwise, also for the owner and for friends who
  may watch). A clip is made to be passed on; friends-only and private
  broadcasts are not.
- The upload belongs to the channel's owner, so the person who cut the clip
  cannot attach it to a post, a short or a message of their own through the
  media domain's "my uploads". Its ID (`Clip.asset_id`) is given to the
  clip's creator once it is READY, for `CreateShort` (which checks through
  the clip, not the upload's owner); everyone else gets it empty.
- Not enforced here, because it needs a change in the media domain: a
  dedicated asset purpose for clips (it is `ASSET_PURPOSE_SHORT` today, so
  the *channel owner* could attach a clip's upload wherever a short's upload
  is accepted, which is their own broadcast anyway).
- A source that becomes restricted after a clip was cut hides the clip (it
  follows its source's visibility); the upload stays until the clip is
  deleted.

The archive upload (`ASSET_PURPOSE_LIVE_ARCHIVE`, owned by the channel's
owner) is held by the video it becomes.

## Realtime pushes

`LiveEvent` (carried as a `tokuly.realtime.v1.Event`):

| Event | Sent to | When |
|---|---|---|
| `stream_updated` | `live:<id>`, the owner, the channel's widgets | went on air, ended, was edited; archive finished (owner only) |
| `stats` | `live:<id>` | the viewer count changed at a sample; a like |
| `chat_message` | `live:<id>` but the people in a block relation with its author (below), chat boxes | chat |
| `chat_message_deleted`, `chat_user_cleared` | `live:<id>`, chat boxes | chat |
| `chat_pin_changed` | `live:<id>` but the people in a block relation with the pinned line's author | moderation |
| `chat_settings_changed` | `live:<id>` | moderation |
| `poll_updated` | `live:<id>`, poll widgets | opened, votes (at most once a second), closed |
| `raid` | `live:<id>` of the raiding stream | it ended with a raid |
| `chat_restriction_changed` | the restricted user | timeout, ban, lifted |
| `encoder_state` | the owner | encoder connected / went away |
| `clip_updated` | the clip's creator | READY or FAILED |
| `alert`, `goal_progress`, `overlay_changed` | the widget's topic | gifts, new subscribers, raids, test alerts; goal moved; configuration changed or token rotated (`revoked`) |

Topics and their guards (registered in `Live::new`):

- `live:<stream id>`: whoever may open the stream (guests for PUBLIC,
  NO_FEED and UNLISTED streams).
- `overlay:<topic key>`: a widget's topic. The topic key is derived from the
  widget's token with a keyed hash (`live_overlays.topic_key`) and returned by
  `GetOverlay.realtime_topic`; the token itself never appears in a topic name
  (topic names travel in frames and logs), and the raw token is not a valid
  topic. After a rotation the key changes with the token and nothing is
  published to the old topic any more (other processes notice within 5 s);
  after `overlay_changed { revoked }` the old topic is closed, so the page
  behind it is let go (`ServerFrame.unsubscribed`).

Chat lines and blocks: a chat line (and a pin, for the pinned line) is
one publish to `live:<id>` carrying an exception list
(`Live::chat_exceptions`). The list travels in the bus message's
`Tokuly-Except` header, never in the frame; every realtime gateway skips the
connections signed in as one of those people
(`tokuly_core::realtime::to_topic_except`). It holds at most
`tokuly_core::realtime::MAX_EXCEPT` (1000) people, filled in this order:

- **The people the author blocked** — always. This part is the author's own
  doing, so it fails closed: an author who blocked more than 1000 people
  gets no live push at all; their lines appear on the next read.
- **The people who blocked the author** — while the whole list fits.
  Anybody can add to this part (throw-away accounts blocking someone), so it
  never stops delivery: past the cap they are left out and the line is
  still published. Then the second line of defence is what keeps them from
  seeing it: the web app drops pushed lines and pins of people the viewer
  blocked (and hides such a pin when the page loads with it).
- **Not the streamer's relations**: the topic guard refuses anyone in a block
  relation with the streamer when they subscribe.

The lists come from the social domain (who blocked whom, read once per
line); nothing is read per viewer. A line whose list cannot be read is not
pushed live. Lists, history and replay filter by the reader's blocks as
before.

Someone who loses access after subscribing loses the subscription too
(`ServerFrame.unsubscribed`, see `docs/events/realtime.md`): a block
between them and the streamer and an unfriending (both re-check the two
people's `live:` subscriptions), a stream narrowed to friends or members
(not a widening, nor a move between the visibilities open to everyone)
or suspended (every subscriber re-checked), a deleted stream (the topic
closes). Known limits: other events on the topic (`stream_updated`,
`stats`, polls) are not filtered by blocks. A block made between reading the
lists and the publication (milliseconds) can let that one line through.

## Moderators

An appointed moderator acts on messages and people: delete, pin, time out,
ban, banned words, end a poll. They answer to the streamer:

- they cannot delete the streamer's messages (`CANNOT_MODERATE_STREAMER`);
- chat-wide settings (`UpdateChatSettings`: on/off, audience, slow mode, link
  policy, stamps) are the streamer's alone (`NOT_THE_STREAMER`), so a
  moderator cannot close the chat on its owner;
- they cannot restrict the streamer or each other, and a moderator who is
  timed out or banned by the streamer is checked under the same lock as
  anyone else: a message sent at the moment of the ban does not get through.

## Overlay widgets

A widget page is opened by its token (`/overlay/<token>`) inside the
streamer's OBS. The token opens `GetOverlay`; the realtime topic is derived
from it (above). Custom CSS may style the widget but must not make that
browser fetch anything: `@import`, `url(…)`, `image-set(…)`, `@font-face`,
`src:`, `expression(…)` and backslash escapes (which can spell any of those)
are refused with `OVERLAY_CSS_UNSAFE`, as is `</style`.

## API tokens

What a token of the developer platform needs for each of the 57 RPCs is listed
in `tokuly_core::api_access`; the scopes are `live:read` and `live:write`
(marked sensitive: `live:read` shows who is watching and who is restricted in
which chat).

| What | Needs |
|---|---|
| reading streams, chat, replay, polls, clips, gift tiers, top supporters, presets | `live:read` |
| `ListViewers`, `ListModerators`, `ListChatRestrictions`, `GetBannedWords`, the analytics RPCs | `live:read` |
| `ListFriendStreams` | `live:read` + `social:read` |
| `ListSentGifts`, `ListReceivedGifts` | `live:read` + `wallet:read` |
| creating, editing, ending and deleting streams, reminders, `ReportWatching`, chat, polls, clips, `SendTestAlert` | `live:write` |
| chat moderation: delete, pin, restrict, banned words, chat settings | `live:write` |
| `SendGift` | `live:write` + `wallet:write` |
| `GetOverlay` | `live:read` (and the widget's token, which the request carries) |
| `GetIngestInfo`, `RotateStreamKey`, `StartStream`, `RaidStream` | a session |
| `AddModerator`, `RemoveModerator` | a session |
| `ListOverlays`, `CreateOverlay`, `UpdateOverlay`, `DeleteOverlay`, `RotateOverlayToken` | a session |

A session, because: the stream key and the ingest URLs are the credential to
broadcast as the channel; going on air and raiding speak to an audience in the
streamer's name and cannot be taken back; appointing a moderator hands power
to another account that outlasts the token; and a widget's token is a secret
that every overlay-management answer contains.

Playback tokens never go to an API token either. `GetStream`, `ReportWatching`
and the answers of the owner's RPCs carry `playback` for a token caller only
when the stream is on air and open to everyone (plain URLs); for a restricted
stream, and for the owner's preview, `playback` is unset
(`Live::playback_for`).

## Follow-ups for the web app

Things this domain relies on the web client (or the edge) to do:

- **The overlay page (`/overlay/<token>`) must send
  `Referrer-Policy: no-referrer`** (and should not load third-party
  resources at all): its URL is the widget's secret.
- Subscribe to the topic in `GetOverlay.realtime_topic`; do not build a topic
  name from the token (that topic does not exist).
- Take new playback URLs from every `ReportWatching` answer (`playback`) and
  use them the next time a media session is opened (reconnect, quality
  switch, rewind): tokens of restricted streams live 15 minutes.
- When a restricted stream's media session ends unexpectedly, call
  `GetStream` before reconnecting: the viewer may have lost access.
- Show `GIFT_RETURNED` as "the stream is gone, your coins are back", not as
  a failure to retry.
- Wallet history: an entry with `counterpart_hidden` has no counterpart to
  show; label it from `reason` and `memo_json` (the channel).
- The edge must refuse `/api/hooks/*` (see "Who may call the hook").

## Notification kinds

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `live.reminder` | people who asked (`SetReminder`) | inbox + push | When the stream goes on air. No actor; title = channel name. Collapse key `live.started:<stream_id>`. `data = { stream_id, short_id, channel_id, channel_handle, channel_name, channel_icon_url }`. |
| `live.started` | subscribers; friends of the owner for friends-only streams | level ALL: inbox + push; PERSONALIZED: inbox; NONE: nothing; friends: inbox + push | Same shape and collapse key. Only PUBLIC and FRIENDS streams; friends-only streams go to subscribers who are friends, then to the other friends with the owner as actor. Not sent to people who got the reminder. |
| `live.gift` | the streamer | inbox + push | Only while their dashboard is closed. Actor: the sender. Collapsed per stream. `data = { stream_id, short_id, gift_id, coins, tier }`. |
| `live.moderator` | the appointed user | inbox + push | No actor; collapsed per channel; once per appointment. |

## Activity entries

| Kind | Audience | When |
|---|---|---|
| `live.started` | friends of the owner | A PUBLIC or FRIENDS stream went on air. Subject `(SUBJECT_TYPE_LIVE_STREAM, id)`, path `/live/<short_id>`, image = thumbnail, `data = { stream_id, short_id, channel_id, channel_handle, channel_name }`. |

## Search documents

| Kind | Indexed while | Fields |
|---|---|---|
| `live` | PUBLIC, upcoming or on air, not suspended | `title`, `text` = description, `tags` = tags + game, `path` = `/live/<short_id>`, `created_at` = start (or schedule), `score` = viewers as of the last edit, `extra = { short_id, channel_id, channel_handle, channel_name, status, category, game, scheduled_at, started_at }` |
| `clip` | READY and its source is public | `title`, `path` = `/clip/<short_id>`, `image_url`, `score` = likes, `extra = { short_id, channel_id, duration_seconds }` |

## Subjects

| Subject type | Resolves when | Owner | Notes |
|---|---|---|---|
| `SUBJECT_TYPE_LIVE_STREAM` (UUID or short ID) | the viewer may open the stream; also for a deleted stream while it is kept | channel owner | `like_count` is kept in `live_streams.like_count` and pushed as `stats`. No interactions on deleted or cancelled streams. |
| `SUBJECT_TYPE_CLIP` (UUID or short ID) | the viewer may watch the clip's source and the clip is READY (always for its creator and the channel owner) | the creator | `like_count` in `live_clips.like_count`. |
| `SUBJECT_TYPE_LIVE_CHAT_MESSAGE` (UUID) | the viewer may open the stream; also for removed messages and deleted streams while kept | the author | For reports only: `interactions_enabled` is false. Server notices do not resolve. |

## Analytics events

Subject `("live_stream", id)`, `owner_id` = the channel's owner.

| Name | Value / props |
|---|---|
| `live.view` | 1; `{ channel_id, viewer }` — a viewer's first heartbeat |
| `live.watch` | seconds since the previous heartbeat; `{ channel_id, viewer }` |
| `live.chat` | 1; `{ channel_id, kind }` |
| `live.gift` | coins; `{ channel_id, tier, net }` |

`viewer` is the account ID or `guest:<keyed hash>`. The streamer's own
heartbeats are not tracked. No ClickHouse table of its own.

## Background work

| Worker | Cadence | What |
|---|---|---|
| event consumer `live` | — | See "Events consumed". |
| job `live.announce` `{ stream_id, phase, after }` | — | One batch (`TOKULY_LIVE_ANNOUNCE_BATCH_SIZE`) of reminders / subscribers / friends, then queues the next. |
| job `live.archive` `{ stream_id }` | — | Joins the segments from going on air to the end (stream copy), `create_asset_from_file`, `videos::create_from_asset` (same title, description, tags, visibility; `VIDEO_ORIGIN_LIVE_ARCHIVE`), links the stream's clips to the video, deletes the segments. Resumable; FAILED keeps the recording for a retry. |
| job `live.clip` `{ clip_id }` | — | Cuts a clip with ffmpeg (stream copy when the cut starts on a keyframe, otherwise re-encode) and stores it as an asset. |
| reconciler | `TOKULY_LIVE_RECONCILE_INTERVAL_SECONDS` | Follows MediaMTX, ends abandoned and over-long streams, samples viewers. One worker at a time (Valkey lock). |
| polls | 1 s | Closes polls whose time is up; pushes held-back vote counts. |
| sweep | 5 min | Recordings nobody needs; trims the recording of a stream on air that will not be archived to its DVR window; requeues archives stuck for 15 min; fails clips stuck for 30 min; erases deleted streams past their retention; finishes interrupted gifts. |

## Valkey keys

| Key | Type | Purpose |
|---|---|---|
| `live:watching:<stream>` | sorted set, TTL 10 min | viewers by last heartbeat |
| `live:dashboard:<stream>` | marker, TTL 3 report intervals | the streamer has their dashboard open |
| `live:slow:<stream>:<user>` | marker, TTL slow mode | slow mode |
| `live:poll-push:<poll>`, `live:polls:dirty` | marker 1 s, set | vote pushes at most once a second |
| `live:archive:<stream>` | lock | one archive attempt at a time |
| `live:reconcile` | lock, TTL 60 s | one reconciler at a time |
| `live:media-down-since` | timestamp | since when MediaMTX does not answer |
| `live:sessions:<stream>` | hash, TTL 1 day | media session ID → the viewer whose playback token opened it (`-`: nobody's) |
| `live:may-watch:<stream>:<viewer>:<visibility>:<epoch>` | marker, TTL 10 s | this viewer may watch (asked by the hook) |
| `live:access-epoch:<owner>` | counter, TTL 1 day | bumped when who may watch the owner's streams changes |
| `live:guests:<stream>:<address group>` | sorted set, TTL 10 min | the (at most 5) anonymous viewers of one address counted in the audience |
| `live:hook-failures:<address>` | counter, TTL 1 min | wrong hook secrets presented by a peer |

## Data export

Exporter `live`: streams (without keys), chat messages, gifts sent and
received, clips, overlays (without tokens), banned words, moderator seats,
chat restrictions, reminders, poll votes.

## Configuration (`LiveConfig`)

| Variable (flag) | Default | Meaning |
|---|---|---|
| `TOKULY_MEDIAMTX_API_URL` (`--mediamtx-api-url`) | empty | MediaMTX control API, e.g. `http://mediamtx:9997`. Empty: streams can be managed but `GetIngestInfo` / `StartStream` answer `unavailable` `MEDIA_SERVER_NOT_CONFIGURED`. |
| `TOKULY_LIVE_HOOK_SECRET` (`--live-hook-secret`) | empty | What MediaMTX presents to the hook (in `authHTTPAddress`); required (16+ characters) when the API URL is set. |
| `TOKULY_MEDIAMTX_API_SECRET` (`--mediamtx-api-secret`) | empty | What the worker presents to MediaMTX's control API; required (16+ characters, different from the hook secret) when the API URL is set. |
| `TOKULY_LIVE_RTMP_URL` | `rtmp://localhost:1935` | Public RTMP ingest. |
| `TOKULY_LIVE_SRT_URL` | `srt://localhost:8890` | Public SRT ingest. |
| `TOKULY_LIVE_WHIP_URL` / `TOKULY_LIVE_WHEP_URL` | `http://localhost:8889` | Public WebRTC publish / playback. |
| `TOKULY_LIVE_HLS_URL` | `http://localhost:8888` | Public LL-HLS playback. |
| `TOKULY_LIVE_PLAYBACK_URL` | `http://localhost:9996` | Public DVR (MediaMTX playback server). |
| `TOKULY_LIVE_RECORDINGS_DIR` | `/recordings` | The directory MediaMTX records into, as the worker sees it. |
| `TOKULY_LIVE_RECONCILE_INTERVAL_SECONDS` | `3` | Cadence of the reconciler. |
| `TOKULY_LIVE_RECONNECT_GRACE_SECONDS` | `60` | How long an encoder may stay away. |
| `TOKULY_LIVE_MAX_HOURS` | `24` | Longest stream. |
| `TOKULY_LIVE_DVR_WINDOW_SECONDS` | `3600` | Rewind window. |
| `TOKULY_LIVE_RECORDINGS_MAX_AGE_HOURS` | `48` | How long an unclaimed or failed recording is kept. |
| `TOKULY_LIVE_DELETED_RETENTION_DAYS` | `30` | How long a deleted stream and its chat are kept. |
| `TOKULY_LIVE_MAX_OPEN_STREAMS` | `20` | Streams per channel that are not over. |
| `TOKULY_LIVE_REPORT_INTERVAL_SECONDS` | `15` | Heartbeat cadence asked of players. |
| `TOKULY_LIVE_ANNOUNCE_BATCH_SIZE` | `500` | Recipients per `live.announce` job. |
| `TOKULY_LIVE_FFMPEG_PATH` / `TOKULY_LIVE_FFPROBE_PATH` | `ffmpeg` / `ffprobe` | Tools for archives and clips. |

Fixed limits: chat message 200 characters; 200 banned words of 30; timeout
10 s to 14 days; 50 moderators; poll question 100, 2–5 options of 50,
15 s–30 min; gifts 10–100 000 coins in 12 tiers (`tokuly_live::tiers`); clip
0.1–60 s, title 64; 20 overlays per channel, custom CSS 50 000; scheduling at
most 366 days ahead.

## Rate limits

| Key | Limit |
|---|---|
| `live-create:<user>` | 30 / hour |
| `live-update:<user>` | 60 / minute |
| `live-key:<user>` | 10 / hour |
| `live-start:<user>` | 30 / minute |
| `live-reminder:<user>` | 60 / minute |
| `live-watch:<stream>:<viewer>` | 20 / minute |
| `live-lists:<user>` | 60 / minute (`ListSubscribedStreams`, `ListViewers`) |
| `live-raid:<user>` | 5 / hour |
| `live-chat:<stream>:<user>` | 8 / 10 seconds (plus slow mode) |
| `live-moderate:<user>` | 120 / minute (delete, pin, settings, banned words, restrict, end poll) |
| hook failures per peer address | 10 wrong secrets / minute, then `429` |
| `live-appoint:<user>` | 20 / hour |
| `live-replay:<user or ip>` | 240 / minute |
| `live-poll:<user>` / `live-vote:<user>` | 30 / hour, 60 / minute |
| `live-gift:<user>` | 30 / minute |
| `live-clip:<user>` / `live-clip-edit:<user>` | 20 / 10 minutes, 60 / minute |
| `live-overlay:<ip>` | 120 / minute (`GetOverlay`) |
| `live-overlay-edit:<user>` / `live-test-alert:<user>` | 120 / minute, 30 / minute |
| `live-dashboard:<user>` / `live-analytics:<user>` | 120 / minute, 60 / minute |
