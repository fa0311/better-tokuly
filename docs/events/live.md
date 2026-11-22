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
owned by `creator_id` and held by `live:clip:<clip_id>`; a domain that reuses
it attaches its own holder.

**Wiring the server has to do** (`crates/server/src/app.rs`):

- construct `Live::new(core, identity, cfg, wallet)` in every process (it
  registers the guards of the `live:` and `overlay:` realtime topics, which
  the gateway needs), `register` it, `spawn_workers` in worker processes, add
  `LiveConfig` to `DomainConfigs`;
- mount `live.http_routes()` at `/hooks/live` on the API listener. **This
  route is for the media server only and must not be reachable through the
  edge**: the Caddyfile forwards `/api/*` to the API with the prefix stripped,
  so `/api/hooks/*` has to be refused there. The route also checks the shared
  secret itself;
- add MediaMTX to `compose.yaml` with `infra/mediamtx/mediamtx.yml`, a volume
  for `/recordings` shared with the worker (`TOKULY_LIVE_RECORDINGS_DIR`),
  `MTX_AUTHHTTPADDRESS=http://api:8081/hooks/live/auth?secret=<TOKULY_LIVE_HOOK_SECRET>`
  and `MTX_WEBRTCADDITIONALHOSTS=<public host>`; publish 1935 (RTMP),
  8890/udp (SRT), 8888 (HLS), 8889 + 8189/udp (WebRTC) and 9996 (DVR), not
  9997 (control API). The worker needs `ffmpeg` and `ffprobe`.

## The media server

Every stream is the MediaMTX path `live/<short_id>`. MediaMTX asks
`POST /hooks/live/auth` (`authMethod: http`) before anyone publishes, reads,
uses the playback server or its control API:

| `action` | Allowed when |
|---|---|
| `publish` | the path is a stream that is not over, deleted or suspended, and the credential (`token`, or `password` for SRT) is its stream key |
| `read` | the stream is on air and PUBLIC / NO_FEED / UNLISTED; otherwise the request carries a valid playback token (`?t=`, or as bearer token) |
| `playback` | as `read`, DVR is on for the stream, and `start` lies within `TOKULY_LIVE_DVR_WINDOW_SECONDS` |
| `api`, `metrics`, `pprof` | user `tokuly` with the hook secret as password (the worker) |

Stream keys are stored encrypted and hashed; playback tokens are a keyed hash
of the stream name and an expiry (6 h), handed out by `GetStream` to whoever
may watch a restricted stream, and to the owner for the preview before going
on air.

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
| `social.user.blocked` | Moderator appointments and reminders between the two are removed. (A blocked user does not see the blocker's streams at all, immediately.) |

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

The wallet also sends its own `wallet.received` notification for the
transfer; `live.gift` is additional and only sent while the streamer's
dashboard is closed.

## Media assets

| Holder | Asset |
|---|---|
| `live:stream:<stream_id>` | the stream's thumbnail (`ASSET_PURPOSE_THUMBNAIL`) |
| `live:clip:<clip_id>` | the clip's upload (`ASSET_PURPOSE_SHORT`, owned by the creator, created with `create_asset_from_file`) |

The archive upload (`ASSET_PURPOSE_LIVE_ARCHIVE`, owned by the channel's
owner) is held by the video it becomes.

## Realtime pushes

`ServerFrame.live` (`LiveEvent`):

| Event | Sent to | When |
|---|---|---|
| `stream_updated` | `live:<id>`, the owner, the channel's widgets | went on air, ended, was edited; archive finished (owner only) |
| `stats` | `live:<id>` | the viewer count changed at a sample; a like |
| `chat_message`, `chat_message_deleted`, `chat_user_cleared` | `live:<id>`, chat boxes | chat |
| `chat_pin_changed`, `chat_settings_changed` | `live:<id>` | moderation |
| `poll_updated` | `live:<id>`, poll widgets | opened, votes (at most once a second), closed |
| `raid` | `live:<id>` of the raiding stream | it ended with a raid |
| `chat_restriction_changed` | the restricted user | timeout, ban, lifted |
| `encoder_state` | the owner | encoder connected / went away |
| `clip_updated` | the clip's creator | READY or FAILED |
| `alert`, `goal_progress`, `overlay_changed` | the widget's topic | gifts, new subscribers, raids, test alerts; goal moved; configuration changed or token rotated (`revoked`) |

Topics and their guards (registered in `Live::new`):

- `live:<stream id>`: whoever may open the stream (guests for PUBLIC,
  NO_FEED and UNLISTED streams).
- `overlay:<token>`: whoever presents a widget's token. After a rotation
  nothing is published to the old topic any more (other processes notice
  within 5 s).

Known limits: the gateway checks a subscription once, so someone who is
blocked or loses access keeps an open subscription until they reconnect; and
a chat push goes to the whole topic, so two viewers who blocked each other
still receive each other's lines live (lists and history filter them).

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

## Data export

Exporter `live`: streams (without keys), chat messages, gifts sent and
received, clips, overlays (without tokens), banned words, moderator seats,
chat restrictions, reminders, poll votes.

## Configuration (`LiveConfig`)

| Variable (flag) | Default | Meaning |
|---|---|---|
| `TOKULY_MEDIAMTX_API_URL` (`--mediamtx-api-url`) | empty | MediaMTX control API, e.g. `http://mediamtx:9997`. Empty: streams can be managed but `GetIngestInfo` / `StartStream` answer `unavailable` `MEDIA_SERVER_NOT_CONFIGURED`. |
| `TOKULY_LIVE_HOOK_SECRET` (`--live-hook-secret`) | empty | Shared with MediaMTX; required (16+ characters) when the API URL is set. |
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
| `live-guests:<ip>` | 60 new anonymous viewers / 10 minutes |
| `live-lists:<user>` | 60 / minute (`ListSubscribedStreams`, `ListViewers`) |
| `live-raid:<user>` | 5 / hour |
| `live-chat:<stream>:<user>` | 8 / 10 seconds (plus slow mode) |
| `live-moderate:<user>` | 120 / minute (delete, pin, settings, banned words, restrict, end poll) |
| `live-appoint:<user>` | 20 / hour |
| `live-replay:<user or ip>` | 240 / minute |
| `live-poll:<user>` / `live-vote:<user>` | 30 / hour, 60 / minute |
| `live-gift:<user>` | 30 / minute |
| `live-clip:<user>` / `live-clip-edit:<user>` | 20 / 10 minutes, 60 / minute |
| `live-overlay:<ip>` | 120 / minute (`GetOverlay`) |
| `live-overlay-edit:<user>` / `live-test-alert:<user>` | 120 / minute, 30 / minute |
| `live-dashboard:<user>` / `live-analytics:<user>` | 120 / minute, 60 / minute |
