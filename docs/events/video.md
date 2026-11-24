# video — events, jobs and configuration

Crate `tokuly-video`, migration `0800_video.sql`, API `tokuly.video.v1`
(`ChannelService`, `VideoService`, `PlaybackService`, `PlaylistService`,
`VideoFeedService`, `WatchPartyService`, `StudioAnalyticsService`, realtime
`VideoEvent`). Depends on `identity`, `social` and `media`.

## API for other domains

```rust
// Channels (live, shorts, engagement).
tokuly_video::channels::summaries(core, &ids) -> AppResult<HashMap<Uuid, ChannelSummary>>      // unknown IDs are absent
tokuly_video::channels::require_owned(core, user_id, channel_id) -> AppResult<ChannelRow>      // CHANNEL_NOT_FOUND unless it is theirs
tokuly_video::channels::default_for(core, user_id) -> AppResult<Option<ChannelRow>>            // the posting identity
tokuly_video::channels::owned_by(core, user_id) -> AppResult<Vec<ChannelRow>>                  // default first
tokuly_video::channels::find(core, "uuid | handle | @handle") -> AppResult<Option<ChannelRow>>
tokuly_video::channels::subscriber_ids(core, channel_id, NotifyLevel::All) -> AppResult<Vec<Uuid>>   // level >= the given one
tokuly_video::channels::is_subscribed(core, user_id, channel_id) -> AppResult<bool>
tokuly_video::channels::set_verified(core, channel_id, bool) -> AppResult<()>                  // staff tooling

// Videos.
tokuly_video::videos::find(core, &Ref) -> AppResult<Option<VideoRow>>                          // UUID or short ID, any state
tokuly_video::videos::create_from_asset(core, NewVideo { .. }) -> AppResult<Uuid>              // e.g. a live archive
tokuly_video::access::Viewer::load(core, Option<user_id>) -> AppResult<Viewer>                 // then viewer.can_watch(&video.facts())

// Game title suggestions (a stream went on air with this game).
tokuly_video::games::record_use(core, title) -> AppResult<()>
```

`NewVideo::new(channel_id, asset_id, title)` is a private draft; set
`visibility`, `origin` (`VIDEO_ORIGIN_LIVE_ARCHIVE`), `origin_id` (the stream)
and `publish: true` to have it go live as soon as the asset is READY. The
asset must be a video owned by the channel's owner, processing or ready, and
not used by another video (`ASSET_ALREADY_USED`).

A channel never reveals which account owns it: show `ChannelSummary`, not
the owner. `ChannelSummary.verified` is the channel's own badge or that of
the owning account.

**Wiring the server has to do** (`crates/server/src/app.rs`):

- construct `Video::new(core, identity, cfg)` in every process (it registers
  the `party` topic guard, which the gateway needs), `register` it,
  `spawn_workers` in worker processes, and add `VideoConfig` to
  `DomainConfigs` (the migrator embeds `0800_video.sql` on its own);
- `video.set_party_stamps(Arc<dyn tokuly_video::PartyStamps>)` with an adapter
  that calls `tokuly_stamp::ensure_usable` and then
  `tokuly_stamp::record_send(.., SendContext::Chat)` (or a new context):
  video may not depend on stamp, so until this is wired stamps in party chat
  are refused with `STAMP_NOT_USABLE`;
- engagement's `ChannelDirectory` can be backed by `channels::require_owned`
  and `channels::summaries`.

## Audiences and locked accounts

Who may watch is decided by `tokuly_video::access` (shared with the shorts
and live domains), on top of the social graph's audiences
(`tokuly_social::graph::AudienceViewer`, loaded once per request; the SQL
predicates refer to the viewer as `$1`–`$6`, see the module docs):

| visibility | who besides the owner may watch (never across a block, never content of a suspended account) |
|---|---|
| PUBLIC (1), NO_FEED (2), UNLISTED (3) | everyone with the link — for a **locked** owner only followers and friends |
| FOLLOWERS (6) | followers and friends |
| FRIENDS (4) | friends; with `allow_link` also anyone with the link (a locked owner's: followers and friends) |
| CLOSE_FRIENDS (7) | the owner's close friends who still are a friend or follower; the visibility itself is their mark |
| CIRCLE (8, `circle_id`) | members of that circle of the owner (same); to everyone but the owner it reads as FOLLOWERS, so members never learn about the circle |
| PRIVATE (5) | nobody |

A circle must be one of the owner's when the content is saved
(`CIRCLE_NOT_FOUND`, `CIRCLE_REQUIRED`). A deleted circle reaches nobody.

**Locked accounts.** Whether an account is locked is copied onto this
domain's rows (`owner_locked`), kept by the handler of
`social.follow.settings_changed` and set from the social domain when the
row is created (re-checked after the commit). Only content of unlocked
owners is in search, trending, recommendations and other lists for
everyone. Reads do not wait for the event: rows that reach a viewer only
because the copy says "unlocked" (open content, viewer neither owner nor
tied) are checked against the social domain on the same request
(`Viewer::drop_newly_locked`, `can_watch_now`), so a lock applies at the
next read. An unlock applies when its event is handled.

**Channels of a locked account** stay visible as channels: a channel
never reveals which account owns it, and hiding it would tell everyone
who knows the account's state which channel is theirs. To anyone who is
neither the owner nor a follower or friend it looks like a channel
without content: no videos, shorts or streams on its pages, no video
count, no trailer, no playlists, not among recommended channels (which
need a public video of an unlocked owner). Its view and subscriber counts
are the real ones: a fixed 0 would mark it as a locked account's
channel. Followers and friends
see it as before. A stranger who subscribed keeps the subscription and
hears about nothing until the account is unlocked.

Playlists keep their own three visibilities (PUBLIC, UNLISTED, PRIVATE);
the public and unlisted playlists of a locked owner reach only followers
and friends and leave search. Watch parties of a video end when its
audience narrows (another audience that may leave members out, or a lock),
since their members are strangers to the owner as often as not.

## Events emitted

Payload structs live in `tokuly_video::events`. All are written in the
transaction of the change they describe.

| Topic | Payload | When |
|---|---|---|
| `video.channel.created` / `.updated` / `.deleted` | `{ channel_id, owner_id, handle }` | A channel was created, edited (handle, name, art, links, trailer) or deleted with its content. Actor: the owner, or nobody for takedowns and account deletion. |
| `video.channel.subscribed` / `.unsubscribed` | `{ channel_id, owner_id, user_id }` | A subscription began or ended (also when a block ends it). Not emitted for a level change, nor for the subscriptions that vanish with a deleted channel or account. |
| `video.video.created` | `VideoEvent` | A draft was created. |
| `video.video.published` | `VideoEvent` | A video went live, or a live video became visible to an audience for the first time. `first_publication` is true exactly once per video: when it is first both PUBLISHED and PUBLIC or FRIENDS. |
| `video.video.unpublished` | `VideoEvent` | A published video went back to draft. |
| `video.video.visibility_changed` | `VideoEvent` + `{ visible_to, changed_at }` | A published video got another audience and stays published, or (once per published open video) its owner locked or unlocked their account. `visibility` is the audience now in effect (a PUBLIC video, or a FRIENDS one with a link, of a locked owner reads as 6, followers; NO_FEED and UNLISTED keep their number, so the feed never shows their entries to friends because of a lock). `visible_to`: who besides the owner may still watch it — `null` when everyone may; otherwise everyone in the audience as the social graph gives it, people in a block with the owner left out; for a lock only the channel's subscribers and the video's raters among the owner's followers and friends (who may have something about it in their inbox). A concurrent change in between names nobody. At most 5 000 people are named (`tokuly_core::events::MAX_VISIBLE_TO`): a longer list is narrowed to the people this domain may have told about it, or to nobody (`[]`), which withdraws too much, never too little. `changed_at`: when the change was made, taken while the row was locked. Not emitted when nothing changed, nor with a publication (that is `video.video.published`). **Never forward to outside parties: `visible_to` is the owner's follower list.** |
| `video.video.deleted` | `VideoEvent` | A video is gone: deleted by the owner, with its channel or account, by a takedown, or because its upload was deleted. Consumers drop what they keep about it (comments, clips). |
| `video.playlist.deleted` | `{ playlist_id, owner_id }` | A custom playlist is gone. |

`VideoEvent` is `{ video_id, short_id, channel_id, owner_id, visibility, origin, origin_id, first_publication }`
(`visibility` / `origin` as enum numbers).

## Events consumed

Durable consumer `video`. Every handler is idempotent.

| Topic | Reaction |
|---|---|
| `media.asset.ready` | The video playing that asset becomes READY: streams, duration and poster frame are copied in, chapters past the end are dropped, a video waiting to be published goes live. The owner is notified (`video.processing_done`) and pushed `video_updated`. |
| `media.asset.failed` | The video becomes FAILED with the media error; a pending publication is cancelled. The owner is notified (`video.processing_failed`). |
| `media.asset.deleted` | The video playing that asset is deleted; a custom thumbnail falls back to the poster frame; a channel icon or banner is cleared. (Normally the holds prevent this; it happens for forced deletions.) |
| `identity.user.deleted` | The user's channels with all content, their playlists, subscriptions, ratings (counters are corrected), watch history, settings, "not interested" marks and party seats (hosting is handed over or the party ends) are deleted. |
| `identity.user.banned` / `.unbanned` | `channels.suspended` and `videos.suspended` follow the account: while suspended, its channels, videos and playlists read as not found for everyone else and leave the search index. |
| `social.user.blocked` | Subscriptions between the two users are removed in both directions. (Lists, feeds and lookups hide the other's content immediately, without waiting for the event.) |
| `social.follow.settings_changed` `{ user_id }` | Re-reads whether the account is locked (the event is only a hint; late or repeated events give the same result) and copies it onto the account's videos and playlists in batches of 200: search documents (videos, playlists, the pictures of playlists listing them) follow, one `video.video.visibility_changed` per published open video, and for a lock the watch parties of those videos end. |
| `admin.content.removed` | `SUBJECT_TYPE_VIDEO`: the video is deleted and its upload removed by force. `SUBJECT_TYPE_CHANNEL`: the channel is deleted with its content. `SUBJECT_TYPE_PLAYLIST`: the playlist is deleted. Unknown IDs and other subject types are ignored. |
| `video.video.published` (own) | With `first_publication`: queues the first `video.announce` job. |
| `video.playlists.stale` (own, internal) `{ video_id?, playlist_ids }` | Emitted in the transaction that changed a video's public state or thumbnail (or deleted it, naming its public playlists); queues a `video.playlists.search` job. |

## Media assets

Holders (`tokuly_media::assets::{attach, detach, detach_holder}`), always in
the transaction that stores or drops the reference:

| Holder | Asset |
|---|---|
| `video:video:<video_id>` | the video's upload (`videos.asset_id`) and its custom thumbnail (`thumbnail_asset_id`) |
| `video:channel-icon:<channel_id>` / `video:channel-banner:<channel_id>` | channel art |

Deleting a video detaches and then deletes its upload (unless something else
still holds it — a clip, a short; takedowns delete regardless). Subtitle
uploads (`ASSET_PURPOSE_SUBTITLE`) are only a way to deliver the file: the
track is stored as its own WebVTT object and the upload is deleted.

## Storage

Media bucket, served by the edge with `Cache-Control: public, max-age=300,
s-maxage=0, must-revalidate` (revalidated, so a removed track stops being
served at once; see `docs/events/media.md`, "Caches and CDNs"):
`subtitles/<video_id>/<subtitle_id>-<content hash>.vtt`. Replacing a track's
content writes a new object (new URL) and deletes the old one; deleting a
video removes the prefix.

## Realtime pushes

`VideoEvent`:

| Event | Sent to | When |
|---|---|---|
| `video_updated` | the owner (user-scoped) | processing finished or failed; a scheduled video went live |
| `party_playback`, `party_member_joined`, `party_member_left`, `party_member_updated`, `party_message`, `party_updated`, `party_ended` | the party's topics | see `WatchPartyService` |

Party topics: `party:<party_id>` for signed-in members and
`party:<party_id>:<key>` per guest (`key` is a secret of the guest's seat,
returned by `JoinParty` as `realtime_topic`). The `party` topic guard admits
signed-in subscribers who have a seat, and anyone presenting a key that
belongs to a seat of an open party. Every event is published to the member
topic and to each guest's topic. `party_updated` carries the video card only
when anyone with the link may watch the video; members of a friends-only
party get theirs from `GetParty`.

Subscriptions end with the seat (`ServerFrame.unsubscribed`, see
`docs/events/realtime.md`): after `party_member_left`, a signed-in member's
subscriptions to the member topic are re-checked on all their devices and a
guest's own topic is closed; after `party_ended` every topic of the party is
closed.

Known limit: the gateway checks a subscription once. A signed-in member who
is removed keeps an already open subscription to `party:<id>` until their
connection resubscribes (a removed guest's topic is simply no longer
published to).

## Notification kinds

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `video.published` | subscribers | level ALL: inbox + push; PERSONALIZED: inbox, plus push if they watched the channel in the last 30 days; NONE: nothing | No actor (the channel speaks, not the account). Title = channel name. Collapse key `video.published:<video_id>`. `data = { video_id, short_id, channel_id, channel_handle, channel_name, channel_icon_url }`. Sent for PUBLIC videos and those shared with an audience; a video shared with an audience (or a public video of a locked owner, as the social domain says when the batch is sent) goes to the subscribers in it. Not sent when the video has `notify_subscribers` off. |
| `video.processing_done` | owner | inbox + push | Collapse key `video.processing:<video_id>`; `data.published` says whether it also went live. Path `/studio/videos/<short_id>`. |
| `video.processing_failed` | owner | inbox + push | `data.error` is the media failure reason. |
| `video.subscribed` | channel owner | inbox + push | Actor: the subscriber. Collapsed per channel (`video.subscribed:<channel_id>`). |
| `video.party_invite` | invited friend | inbox + push | Actor: the inviter. Collapsed per party; path `/party/<code>`. |

## Activity entries

| Kind | Audience | When |
|---|---|---|
| `video.published` | friends of the owner; the owner alone for close friends and circle videos | Once per video, when it is first PUBLISHED and PUBLIC or shared with an audience (followers, friends, close friends, a circle). Subject `(SUBJECT_TYPE_VIDEO, id)`, path `/watch/<short_id>`, image = thumbnail, `data = { video_id, short_id, channel_id, channel_handle, channel_name, duration_seconds }`. The audience is friends also for public videos: a channel does not reveal its owner to everyone. |

## Search documents

| Kind | Indexed while | Fields |
|---|---|---|
| `channel` | the owner is not suspended | `title` = name, `text` = description, `tags` = [handle], `path` = `/channel/<handle>`, `score` = subscribers (as of the last edit), `extra = { handle, verified, subscriber_count }` |
| `video` | PUBLISHED, PUBLIC, not suspended, owner not locked | `title`, `text` = description, `tags` = tags + game, `path` = `/watch/<short_id>`, `created_at` = publication time, `score` = views (as of the last edit), `extra = { short_id, channel_id, channel_handle, channel_name, duration_seconds, category, game, age_restricted, origin }` |
| `playlist` | custom, PUBLIC, not suspended, owner not locked | `title`, `text` = description, `path` = `/playlist/<short_id>`, `image_url` = first public video's thumbnail, `extra = { short_id, item_count, channel_id, channel_handle, channel_name }` (the channel's, for a playlist on a channel) |

## Subjects

| Subject type | Resolves when | Owner | Notes |
|---|---|---|---|
| `SUBJECT_TYPE_CHANNEL` (UUID or handle) | the viewer may see the channel | channel owner | |
| `SUBJECT_TYPE_VIDEO` (UUID or short ID) | the viewer may watch the video | channel owner | `interactions_enabled` = comments on and published. `comment_count` is kept in `videos.comment_count`. `like_count` changes are ignored: video likes go through `VideoService.RateVideo`. |
| `SUBJECT_TYPE_PLAYLIST` (UUID or short ID) | the viewer may open the playlist; never for the built-in lists | playlist owner | `like_count` is kept in `playlists.like_count`. |

## Analytics events

All with `owner_id` = the channel's owner; queried by `StudioAnalyticsService`.

| Name | Subject | Value / props |
|---|---|---|
| `video.play` | `("video", id)` | `{ channel_id, session, viewer, source }` — a playback session started |
| `video.watch` | `("video", id)` | value = seconds watched since the last report; props as above plus `from` / `to` (retention buckets 0–99 covered) |
| `video.view` | `("video", id)` | value 1; a counted view |
| `video.like` / `video.dislike` | `("video", id)` | `{ channel_id }` |
| `channel.subscribe` / `channel.unsubscribe` | `("channel", id)` | `{ video_id }` (the video the viewer subscribed from; may be empty) |

`viewer` is the account ID or `ip:<keyed hash>`; `source` is `TrafficSource`
as its number. Sessions of the channel owner on their own videos are not
tracked. No ClickHouse table of its own is needed.

## Background work

| Worker | Cadence | What |
|---|---|---|
| event consumer `video` | — | See "Events consumed". |
| job `video.announce` `{ video_id, after }` | — | One batch (`TOKULY_VIDEO_ANNOUNCE_BATCH_SIZE` subscribers, by user ID after `after`) of `video.published` notifications, then queues the next batch. |
| job `video.playlists.search` `{ video_id?, playlist_ids, after }` | — | Rewrites the search documents of public playlists listing a video (200 per job, by playlist ID after `after`, then the next job), so their picture never shows a video that left the public. Reads without locks; only the write takes the rows `FOR SHARE`, so a playlist that stopped being public meanwhile is skipped. |
| publishing | `TOKULY_VIDEO_PUBLISH_INTERVAL_SECONDS` | Publishes scheduled videos that are due; every 20th run catches up videos whose `media.asset.*` event was missed. |
| trending | `TOKULY_VIDEO_TRENDING_INTERVAL_SECONDS` | `trending_score` = Σ over the last 7 days of each hour's `views + 4 × likes`, halved per `TOKULY_VIDEO_TRENDING_HALF_LIFE_HOURS`; prunes `video_hourly_stats` older than 8 days. |
| parties | 10 s | Drops members without a heartbeat for `TOKULY_VIDEO_PARTY_MEMBER_TIMEOUT_SECONDS` (hosting is handed over), ends parties older than `TOKULY_VIDEO_PARTY_MAX_HOURS`, deletes parties a day after they ended. |

## Valkey keys

| Key | Type | Purpose |
|---|---|---|
| `video:playback:<session_id>` | JSON, TTL 2 h (renewed by reports) | a playback session |
| `video:viewed:<video>:<viewer>` | marker, TTL `TOKULY_VIDEO_VIEW_DEDUPE_SECONDS` | this viewer's view of this video was counted |
| `video:home:<user or "anon">:<token>` | JSON, TTL 30 min | a ranked home feed being paged through |

## Data export

Exporter `video`: channels, videos (with subtitle tracks), playlists with
their video IDs, subscriptions, ratings, watch history, "not interested"
marks and the history setting.

## Configuration (`VideoConfig`)

| Variable (flag) | Default | Meaning |
|---|---|---|
| `TOKULY_VIDEO_MAX_CHANNELS` (`--video-max-channels`) | `5` | Channels per account. |
| `TOKULY_VIDEO_MIN_VIEW_SECONDS` (`--video-min-view-seconds`) | `10` | Watch time after which a session counts as a view (half the duration for shorter videos). |
| `TOKULY_VIDEO_VIEW_DEDUPE_SECONDS` (`--video-view-dedupe-seconds`) | `21600` | Period in which the same viewer's further views of a video are not counted. |
| `TOKULY_VIDEO_REPORT_INTERVAL_SECONDS` (`--video-report-interval-seconds`) | `10` | Progress report cadence asked of clients; three times this is the longest gap that still counts as watching. |
| `TOKULY_VIDEO_PUBLISH_INTERVAL_SECONDS` (`--video-publish-interval-seconds`) | `15` | Cadence of the publishing worker. |
| `TOKULY_VIDEO_TRENDING_INTERVAL_SECONDS` (`--video-trending-interval-seconds`) | `300` | Cadence of the trending refresh. |
| `TOKULY_VIDEO_TRENDING_HALF_LIFE_HOURS` (`--video-trending-half-life-hours`) | `24` | Half-life of a view or like in the trending score. |
| `TOKULY_VIDEO_MAX_PLAYLISTS` (`--video-max-playlists`) | `200` | Custom playlists per account. |
| `TOKULY_VIDEO_MAX_PLAYLIST_ITEMS` (`--video-max-playlist-items`) | `5000` | Videos per playlist. |
| `TOKULY_VIDEO_ANNOUNCE_BATCH_SIZE` (`--video-announce-batch-size`) | `500` | Subscribers per `video.announce` job. |
| `TOKULY_VIDEO_PARTY_MAX_MEMBERS` (`--video-party-max-members`) | `100` | Seats per watch party, guests included. |
| `TOKULY_VIDEO_PARTY_MAX_HOSTED` (`--video-party-max-hosted`) | `3` | Open parties one account may host. |
| `TOKULY_VIDEO_PARTY_MAX_HOURS` (`--video-party-max-hours`) | `12` | Lifetime of a party. |
| `TOKULY_VIDEO_PARTY_MEMBER_TIMEOUT_SECONDS` (`--video-party-member-timeout-seconds`) | `60` | Heartbeat silence after which a member is dropped. |
| `TOKULY_VIDEO_ANALYTICS_MIN_AUDIENCE` (`--video-analytics-min-audience`) | `5` | Distinct viewers before analytics describe an audience: below it (or when a range differs from the same range one day shorter by fewer viewers), unique viewers, watch time, traffic sources and retention are left out, and days with fewer viewers show no numbers. Best effort against comparing reports. |

Fixed limits (`tokuly_video::validate`, `subtitles`): channel name 50,
description 1000, 5 links; title 100, description 5000, 20 tags of 30
characters, game 100, 100 chapters at least 5 s apart; playlist title 100,
description 1000; 50 subtitle tracks per video of at most 1 MiB and 50 000
cues; guest name 24, party message 500 characters; scheduling at most 366
days ahead.

## Rate limits

| Key | Limit |
|---|---|
| `video-channel-create:<user>` | 10 / hour |
| `video-handle-check:<user or ip>` | 120 / minute |
| `video-subscribe:<user>` | 60 / minute |
| `video-create:<user>` | 30 / hour |
| `video-rate:<user>` | 60 / minute |
| `video-games:<user or ip>` | 240 / minute |
| `video-subtitle:<user>` | 120 / hour (uploads of content) |
| `video-play:<viewer>` | 120 / minute (`StartPlayback`) |
| `video-report:<session>` | 120 / minute (`ReportPlayback`) |
| `video-home:<user or ip>` | 120 / minute (`GetHomeFeed`) |
| `video-playlist-create:<user>` | 30 / hour |
| `video-playlist-items:<user>` | 120 / minute (`AddPlaylistItem` and `RemovePlaylistItem` together) |
| `video-party-create:<user>` | 20 / hour |
| `video-party-preview:<user or ip>`, `video-party-join:<user or ip>` | 60 / minute, 30 / minute |
| `video-party-control:<seat>` | 240 / minute (`SetPartyPlayback`) |
| `video-party-chat:<seat>` | 20 / 10 seconds |
| `video-party-invite:<user>` | 100 / hour |
