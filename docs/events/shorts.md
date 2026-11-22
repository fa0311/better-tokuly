# shorts — events, jobs, ranking and configuration

Crate `tokuly-shorts`, migration `1000_shorts.sql`, API `tokuly.shorts.v1`
(`ShortService`, `ShortFeedService`, `ShortDiscoveryService`,
`ShortAnalyticsService`; no realtime event). Depends on `identity`, `social`,
`media` and `video`. No ClickHouse table of its own.

## API for other domains

```rust
tokuly_shorts::shorts::find(core, &Ref) -> AppResult<Option<ShortRow>>          // UUID or short ID, any state
tokuly_shorts::shorts::find_ref(core, "uuid | short id") -> AppResult<Option<ShortRow>>
tokuly_shorts::access::can_watch(&Viewer, &short.facts()) -> bool              // Viewer is tokuly_video::access::Viewer
tokuly_shorts::Shorts::set_clip_directory(Arc<dyn ClipDirectory>)              // see below
```

**Wiring the server has to do** (`crates/server/src/app.rs`, `bridges.rs`):

- construct `Shorts::new(core, identity, cfg)` after `Video` and before
  `Engagement` (it registers the `SUBJECT_TYPE_SHORT` resolver and the
  `shorts` exporter), `register` it, `spawn_workers` in worker processes, add
  `ShortsConfig` to `DomainConfigs`;
- once the live domain exists, bridge clips:

  ```rust
  #[async_trait]
  pub trait ClipDirectory: Send + Sync + 'static {
      /// The clip (UUID or short ID) if `user_id` created it.
      async fn clip_of(&self, user_id: Uuid, clip_id: &str) -> AppResult<Option<ClipMedia>>;   // { clip_id, asset_id }
  }
  shorts.set_clip_directory(Arc::new(ClipsForShorts { .. }));
  ```

  Until it is wired, "create a short from a clip" only accepts media the
  caller owns (and a clip of theirs that the `SUBJECT_TYPE_CLIP` resolver
  confirms); someone else's asset is always refused.

## Events emitted

Payload `tokuly_shorts::events::ShortEvent`:
`{ id, short_id, channel_id, owner_id, visibility, first_publication, reason }`
(`id` is the UUID, `short_id` the 11-character ID). Written in the
transaction of the change.

| Topic | When |
|---|---|
| `shorts.short.created` | A draft was created. |
| `shorts.short.published` | A short went live, or first reached an audience. `first_publication` is true exactly once per short: when it is first PUBLISHED and PUBLIC or FRIENDS. |
| `shorts.short.deleted` | A short is gone for everyone. `reason`: `owner`, `channel_deleted`, `account_deleted`, `moderation`, `asset_deleted`. Emitted once, also for a short that is only withdrawn (below). |

## Events consumed

Durable consumer `shorts`. Events arrive at least once and not always in
order; every handler gives the same result for a duplicate, a late or a
reordered event, or one about a row that is gone.

| Topic | Reaction |
|---|---|
| `media.asset.ready` / `.failed` | Reads the asset. The draft playing it is PUBLISHED (streams, size, poster copied in), or FAILED with the media error, `SHORT_TOO_LONG` or `SHORT_NOT_VERTICAL` (owner notified). A short waiting for the asset as its cover frame shows it, or falls back to the poster. |
| `media.asset.deleted` | The short playing it is deleted; a cover image falls back to the poster frame. |
| `identity.user.deleted` | The user's shorts are deleted; their saves (counts corrected), copied likes and follows, "not interested" marks, seen list and mentions of them go. |
| `identity.user.banned` / `.unbanned` | Reads `users.banned_at` (the event is only a hint) and sets `shorts.suspended`: the shorts leave or rejoin hashtag and sound counts and the search index. |
| `social.user.blocked` | If the block is still in place: the two lose each other's shorts from their saved and liked lists. Reads hide the other's shorts at once, without the event. |
| `admin.content.removed` | `SUBJECT_TYPE_SHORT`: deleted for good, withdrawn or not; its own upload is deleted whatever holds it. |
| `video.channel.deleted` | The channel's shorts are deleted; follows and marks against it go. |
| `video.channel.subscribed` / `.unsubscribed` | Sets the copy in `short_follows` to what `channels::is_subscribed` says now. A new subscription within an hour of watching one of the channel's shorts credits that short (`short.follow`). |
| `engagement.reaction.changed` | Keeps `short_likes`; the latest change (by event time) wins. A new like counts towards the ranking once per account and week. |
| `engagement.comment.published` | Counts towards the ranking once per commenter and week (never the owner). |
| `engagement.report.created` | Sets `shorts.reported_at`: the owner can no longer delete the short out of moderators' reach. |
| `search.reindex.requested` | `kind` `short` or `hashtag`: queues `shorts.reindex`. |
| `shorts.short.published` (own) | With `first_publication`: queues `shorts.announce`. |

## Deleting, and reported shorts

`DeleteShort` deletes the row, hashtag and sound counts, search document,
saves and statistics, and the upload (unless something else holds it; clip
media is never deleted). A short that has been reported is *withdrawn*
instead (`SHORT_STATUS_WITHDRAWN`): gone for everyone including its owner,
`shorts.short.deleted` emitted, but row and media kept. Moderators (role
`moderator`, never for their own shorts) still open it with `GetShort`. It is
deleted for good `TOKULY_SHORTS_REPORTED_RETENTION_DAYS` after the
withdrawal (not after its creation), or at once by a takedown.

## The for-you ranking

**Base score** per short, written by the ranking worker
(`tokuly_shorts::ranking`):

```text
engagement = Σ over the hourly buckets of the last 7 days:
               (views + 0.5·loops + 4·likes + 6·comments + 8·shares + 6·saves) · 0.5^(bucket age / half-life)
quality    = (Σ watch ratios + 0.5·10) / (samples + 10)      average watch-through, pulled towards 50 % at first
freshness  = 0.5^(age of the short / (3 · half-life))
score      = (1 + ln(1 + engagement)) · (0.4 + 1.2·quality) · (0.25 + 0.75·freshness)
```

Half-life 24 h by default. A short nobody has seen scores 1. `trending` is
the engagement sum alone.

**Signals count once per account.** Per viewer, short and six hours: one
view (a stay of at least a second) and one sample (the first stay: its
watch ratio and up to ten loops). Per account, short and week: one like,
one comment, one save, one share. The owner's own actions never count.
Reporting more, or toggling, moves nothing.

**Candidates** for a viewer: the newest public shorts, the best scored, the
newest from followed channels, the newest from friends' channels
(friends-only included), the best scored carrying a hashtag of shorts the
viewer liked or saved in the last 90 days; up to `feed_batch_size` each.
Never: their own shorts, "not interested" ones, shorts of anyone in a block
with them, suspended authors, and seen shorts (unless nothing else is left:
then the pool is ranked again and items are flagged `repeat`).

**Personal rank**:

```text
rank = score · 1.6 (friend's channel) · 1.4 (followed channel)
             · (1 + 0.1 · Σ min(liked shorts with that hashtag, 3), at most +0.6)
             · 0.35^(shorts of the channel marked "not interested", ≤ 3)
             · 0.7^(hashtags of the short the viewer marked "not interested", ≤ 3)
```

then spread so no channel appears twice within three places. Deterministic.
A ranked batch is stored in Valkey for 30 minutes; the cursor is its token
and an offset, so pages do not shift while shorts arrive. Anonymous and new
viewers get the same feed without the personal factors.

Trending hashtags: the summed `trending` of a hashtag's public shorts; the
top 50 carried by at least two channels.

## Notification kinds

All without an actor (the channel speaks; a channel does not reveal its
account); title = channel name; `data = { id, short_id, channel_id,
channel_handle, channel_name, channel_icon_url, duration_seconds }`.

| Kind | Recipient | Notes |
|---|---|---|
| `shorts.published` | subscribers | ALL: inbox + push; PERSONALIZED: inbox, plus push if they liked or saved a short of the channel in the last 30 days; NONE: nothing. Friends-only shorts go to subscribers who are friends. Collapse key `shorts.published:<id>`. At most `announcements_per_day` shorts per channel and 24 h. Each subscriber once: progress is stored in `shorts.announce_cursor`. |
| `shorts.mention` | mentioned accounts | Once per account and short, when it can first open the short; never across a block. A creator may newly mention 100 accounts an hour. |
| `shorts.processing_failed` | owner | `data.error`: media reason, `SHORT_TOO_LONG`, `SHORT_NOT_VERTICAL`. Path `/studio/shorts/<short_id>`. |
| `shorts.milestone` | owner | 1 000 / 10 000 / 100 000 views; collapse key `shorts.milestone:<id>`; `data.milestone`. |

## Activity entries

`short.published`, audience friends of the owner, once per short when first
PUBLISHED and PUBLIC or FRIENDS. Subject `(SUBJECT_TYPE_SHORT, id)`, path
`/shorts/<short_id>`.

## Search documents

| Kind | Indexed while | Fields |
|---|---|---|
| `short` | PUBLISHED, PUBLIC, not suspended | `title` = caption excerpt (channel name if none), `text` = caption, `tags` = hashtags, `path` = `/shorts/<short_id>`, `score` = views at last change, `extra = { short_id, channel_id, channel_handle, channel_name, duration_seconds }` |
| `hashtag` | a public short carries it | `title` = `#tag`, `score` = short count, `path` = `/shorts/tags/<tag>`, `extra.source = "shorts"`; rewritten by the ranking worker when the count moved |

## Subject

`SUBJECT_TYPE_SHORT` (UUID or short ID) resolves when the viewer may watch
the short. `interactions_enabled` = comments on and published: engagement
has one switch, so a short with comments off takes no new likes either.
`like_count` / `comment_count` follow `counter_changed`; bookmark and share
counters of engagement are ignored (saves and shares are this domain's).

## Media assets

Holder `shorts:short:<id>`: the video, an IMAGE cover upload, and a FRAME
cover (an `ASSET_PURPOSE_THUMBNAIL` asset this domain cuts with ffmpeg from
the original and creates through `tokuly_media::create_asset_from_file`).

## Analytics events (ClickHouse `events`)

Subject `("short", id)`, `owner_id` = channel owner, `props.channel_id`.

| Name | Value / props |
|---|---|
| `short.watch` | seconds played; `viewer`, `source`, `ratio`, `loops`, `completed` (first stay per viewer and window) |
| `short.view` | 1; `viewer`, `source` |
| `short.like`, `short.comment` | ± count delta |
| `short.save` | ±1 |
| `short.share` | 1; `target` |
| `short.follow` | 1 |
| `short.not_interested` | 1 (not shown to creators) |

Creator analytics report watch behaviour (ratio, loops, watch time, plays)
only where at least five distinct viewers stand behind the number.

## Background work

| Worker | Cadence | What |
|---|---|---|
| event consumer `shorts` | — | above |
| job `shorts.announce` `{ short_id, after }` | — | subscriber notifications, 20 batches of `announce_batch_size` per job |
| job `shorts.cover` `{ short_id, offset_ms }` | — | cuts a cover frame (downloads the original, runs ffmpeg) |
| job `shorts.reindex` `{ kind, after }` | — | republishes search documents, 500 per job |
| flush | `flush_interval_seconds` | buffered playback counters to `shorts` and `short_hourly_stats`; view milestones |
| ranking | `ranking_interval_seconds` | scores, trending hashtags, hashtag search documents; every 5th run catches up missed processing results; hourly: prunes old marks and taken-back likes, purges withdrawn shorts |

## Valkey keys

| Key | Purpose |
|---|---|
| `shorts:pending`, `shorts:pending:<short>` | buffered playback counters |
| `shorts:viewed:<short>:<viewer>`, `shorts:stayed:<short>:<viewer>` | view / sample counted (TTL `view_dedupe_seconds`) |
| `shorts:once:<signal>:<short>:<account>` | like / comment / save / share counted (7 days) |
| `shorts:seen:<user>` | sorted set of seen shorts, capped at `seen_cap` |
| `shorts:lastwatch:<user>:<channel>` | short to credit with a follow (1 h) |
| `shorts:foryou:<user or anon>:<token>` | a ranked for-you batch |
| `shorts:ranking:lease` | one ranking run per interval |

## Configuration (`ShortsConfig`)

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_SHORTS_MAX_DURATION_SECONDS` | `180` | Longest short. |
| `TOKULY_SHORTS_MAX_ASPECT_RATIO` | `1.78` | Widest video (width / height). |
| `TOKULY_SHORTS_MIN_VIEW_MS` | `1000` | Playback after which a stay is a view. |
| `TOKULY_SHORTS_VIEW_DEDUPE_SECONDS` | `21600` | Window of one view and one sample per viewer. |
| `TOKULY_SHORTS_SEEN_CAP` / `_SEEN_TTL_DAYS` | `2000` / `30` | Seen list size and idle lifetime. |
| `TOKULY_SHORTS_FLUSH_INTERVAL_SECONDS` | `5` | Counter flush cadence. |
| `TOKULY_SHORTS_RANKING_INTERVAL_SECONDS` | `60` | Ranking cadence. |
| `TOKULY_SHORTS_RANKING_HALF_LIFE_HOURS` | `24` | Signal half-life. |
| `TOKULY_SHORTS_FEED_BATCH_SIZE` / `_FEED_BATCH_TTL_SECONDS` | `200` / `1800` | For-you batch size and lifetime. |
| `TOKULY_SHORTS_ANNOUNCE_BATCH_SIZE` | `500` | Subscribers per notification batch. |
| `TOKULY_SHORTS_ANNOUNCEMENTS_PER_DAY` | `3` | Announced shorts per channel and 24 h. |
| `TOKULY_SHORTS_REPORTED_RETENTION_DAYS` | `30` | How long a withdrawn short is kept. |
| `TOKULY_SHORTS_MAX_SAVES` | `5000` | Saved shorts per account. |
| `TOKULY_SHORTS_FFMPEG_PATH` | `ffmpeg` | Binary for cover frames (worker hosts). |

Fixed limits: caption 500 characters, 30 hashtags of 50, 20 mentions, sound
title / artist 100, 3 pins per channel, 50 shorts per batch get, 50 reports
per call.

## Rate limits

| Key | Limit |
|---|---|
| `shorts-create:<user>` | 30 / hour |
| `shorts-update:<user>` | 120 / hour |
| `shorts-cover:<user>` | 20 / hour (cover frames) |
| `shorts-mention:<user>` | 100 newly mentioned accounts / hour |
| `shorts-save:<user>`, `shorts-hide:<user>` | 120 / minute |
| `shorts-share:<user>` / `:ip:<address>` | 60 / 30 per minute |
| `shorts-report:<viewer>` | 120 / minute |
| `shorts-foryou:<user or ip>` / `shorts-foryou-batch:` | 120 / 30 per minute |
| `shorts-suggest:<user or ip>` | 240 / minute |
| `shorts-analytics:<user>` | 60 / minute |

## Data export

Exporter `shorts`: own shorts (withdrawn included), saved and liked shorts
the user may still open, "not interested" marks, followed channels.

## Known limits

- Counts (`use_count`, `short_count`, `remix_count`) are adjusted per viewer
  for blocks, so a number matches the list behind it.
- A caption edited after a report: the report keeps only the title excerpt
  engagement stored.
- A mention recipient can report the channel, but cannot block the account
  behind it (channels hide their owner): needs a social/video feature.
