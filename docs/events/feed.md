# feed — events, jobs and configuration

Crate `tokuly-feed`, migration `1300_feed.sql`, API `tokuly.feed.v1`
(`ActivityService`, `MapStoryService`, `HomeFeedService`, `NewsService`,
`NewsAdminService`, `RecapService`). Depends on `core`, `proto`, `identity`
and `social`.

## How an activity travels

```
any domain ── outbound::activity(tx, ActivityEntry) ──▶ activity.recorded
                                                            │
                          consumer "feed" (events::consume, worker processes)
                                                            │
                                                     activities (one row)
        ┌───────────────┬──────────────────┬───────────────┼──────────────┐
     my log        profile list      friends' feed     map stories     home feed / recaps
   (everything)   (by audience)    (bursts collapsed)  (located, 48 h)   (typed cards)
```

Nothing is fanned out. Every list is a read of `activities` that decides
*when it is read* what the reader may see:

1. **Audience** of the entry: the owner sees everything; friends see
   `Friends` and `Public`; everyone else (signed in or not) sees `Public`.
2. **Hidden kinds**: a kind (or kind prefix, e.g. `geo`) the owner hid is
   shown to nobody but the owner.
3. **Blocks**: authors hidden from the reader by a block (either direction)
   are left out, and so is any entry that is *about* or *names* such a person
   (`subject` of type USER, `with_user_ids`), because its summary carries
   their name.
4. Entries of banned or deleted accounts are not shown to others.

So a friendship that ends, a block, or a changed preference needs no
clean-up: the next read is already right.

What the *content* behind an entry became is not known at read time, so
the events of the producing domains are followed (`tokuly_feed::withdrawals`,
table below): an entry about content that is gone is deleted (everyone's
entries about it), a note's entries get the note's new audience when its
visibility changes, and an expired story's entry becomes its owner's own
memory (friends can no longer open the story, so its title and cover are
not shown to them either). The latest change per subject is remembered in
`feed_subject_changes` for 8 days, as long as events can still arrive late
or twice: an `activity.recorded` delivered after the change gets the same
treatment, and an older change delivered after a newer one is ignored.

The consumer is idempotent on the event ID (`feed_processed_events`, written
in the same transaction as the entry). The marker also keeps an entry its
owner deleted from coming back on a redelivery.

### What producers should put into an `ActivityEntry`

The entry is stored as given, after sanitizing: the kind must be dotted
lowercase (`[a-z0-9_]` segments, ≤ 64 chars) or the entry is dropped;
summaries become single lines of ≤ 300 chars (a missing language falls back
to the other one); `path` must be an in-app path (`/…`); `image_url` must be
http(s) or root-relative; at most 20 `with_user_ids`; impossible coordinates
and `data` over 8 KB are discarded. `occurred_at` is the event's time.

| Field | Used for |
|---|---|
| `kind` | filters, labels, burst collapsing (same person + same kind), card type |
| `subject` | the card type and link target; duplicate detection in the home feed; `admin.content.removed` |
| `with_user_ids` | "with" avatars; block filtering; bump / new-friend statistics in recaps |
| `location` | makes the entry a **map story** for 48 hours; "areas" in recaps |
| `data.place_name` (or `data.name` on a PLACE subject) | place names in recaps |
| `data.region_code`, `data.region_name` | regions in recaps (entries of kind `geo.new_region…` / `geo.region…`) |

Card types (`ActivityCardType`) are derived in this order
(`tokuly_feed::kinds::card_type`):

1. by kind: `account.joined` → JOINED, `social.bumped` → BUMP,
   `social.friend_added` → FRIENDSHIP, `geo.new_region` / `geo.region` →
   REGION, `geo.new_place` / `geo.place` → PLACE;
2. by subject type: NOTE, STORY, VIDEO, SHORT, LIVE_STREAM → LIVE, CLIP,
   PLAYLIST, STAMP_PACK, CHANNEL, PLACE;
3. by kind family when there is no subject: `note`, `story`, `video`,
   `short`, `live`, `clip`, `playlist`, `stamp.pack`, `channel`, `place`.

Entries without a card type appear in logs and the friends' feed, not in the
home feed. Pairwise events (a bump, a new friendship) should be recorded once
per person with the other person as USER subject, as the social domain does:
the home feed then shows one card when both are the reader's friends.

Kinds with a display name of their own: `account.joined`, `social.bumped`,
`social.friend_added`, `geo.new_region`, `geo.new_place`, `geo.visit`,
`geo.gathering`, `geo.trip`, `stamp.pack.published`; other kinds get the
label of their family (`note`, `story`, `video`, `short`, `live`, `clip`,
`playlist`, `channel`, `stamp`, `chat`, `wallet`, `geo`, `social`,
`account`) or, failing that, the kind itself. Add labels in
`backend/crates/feed/src/kinds.rs`.

## Events consumed

Durable consumer `feed`.

| Topic | Payload | Effect |
|---|---|---|
| `activity.recorded` | `tokuly_core::outbound::ActivityEntry` | stores the entry (see above) |
| `identity.user.created` | `{ user_id, handle, invite_code }` | records `account.joined` (public, 「Tokulyに参加しました！」, path `/u/<handle>`) |
| `identity.user.deleted` | `{ user_id }` | deletes the user's entries, every entry that names or is about them, their hidden kinds, map-story seen marks (as viewer and as author), news clicks, feedback and interests, recap notices |
| `social.friend.removed` | `{ user_id, friend_id, blocked }` | forgets what the two watched of each other's map stories (everything else is filtered at read time) |
| `social.user.blocked` | `{ user_id, blocked_id }` | same |
| `admin.content.removed` | `{ subject_type, subject_id, reason, moderator_id }` | deletes every entry pointing at that subject; a `SUBJECT_TYPE_NEWS_ARTICLE` is taken down instead; `SUBJECT_TYPE_USER` is ignored (a profile takedown does not erase bumps and friendships) |
| `notes.note.deleted` | `{ note_id, … }` | deletes every entry about the note (`SUBJECT_TYPE_NOTE`) |
| `notes.story.deleted` | `{ story_id, … }` | the same for the story (`SUBJECT_TYPE_STORY`) |
| `video.video.deleted` | `{ video_id, … }` | the same for the video (`SUBJECT_TYPE_VIDEO`) |
| `shorts.short.deleted` | `{ id, … }` | the same for the short (`SUBJECT_TYPE_SHORT`) |
| `live.stream.deleted` | `{ stream_id, … }` | the same for the stream (`SUBJECT_TYPE_LIVE_STREAM`) |
| `stamp.pack.deleted` | `{ pack_id, … }` | the same for the pack (`SUBJECT_TYPE_STAMP_PACK`) |
| `notes.note.visibility_changed` | `{ note_id, author_id, visibility }` | the author's entries about the note get the note's audience: public 1 → public, friends 4 → friends, anything else → private |
| `notes.story.expired` | `{ story_id, owner_id }` | the owner's entries about the story become private |

## Events emitted

The feed domain has no events of its own. It uses the outbound requests of
`tokuly_core::outbound`:

| Topic | When |
|---|---|
| `notify.requested` | the monthly recap notice (below) |
| `search.changed` | a news article was stored, changed, hidden (source switched off), taken down or pruned: documents of kind `news` (`title`, `text` = excerpt, `tags`, `path` = `/news/articles/<id>`, `owner_id` = none, `extra` = `{ url, source_id, source_name, language }`) |

## Notification kinds

| Kind | When | Delivery |
|---|---|---|
| `feed.recap_ready` | within the first 3 days after a month ended in the user's time zone, to users with at least `TOKULY_FEED_RECAP_MIN_ACTIVITIES` entries in that month; once per user and month (`feed_recap_notices`) | inbox only, no actor, collapse key `feed.recap:<YYYY-MM-01>`, path `/recap/<YYYY-MM>`, `data = { period, start_date, end_date }`. December's notice announces the year (`/recap/<YYYY>`, `period = "year"`) |

## Realtime

None. (There is no feed event message.) Viewing a map
story is private: `MarkMapStorySeen` emits nothing and tells nobody.

## News

```
news_sources ──(scheduler / RefreshNewsSources)──▶ feed.news.fetch job
                                                       │
     conditional GET (ETag / Last-Modified) within limits, SSRF-guarded
                                                       │
     feed-rs (RSS 0.9x/1.0/2.0, Atom, JSON Feed) → title, link, excerpt, image, date, categories
                                                       │
     news_articles (one row per canonical URL) ──▶ search.changed
```

- **Sources** are managed by admins (`NewsAdminService`, role `admin`;
  `RemoveNewsArticle` also `moderator`) and seeded from `TOKULY_NEWS_FEEDS`
  when a worker starts. At most 500 exist.
- **Fetching is SSRF-safe**: http/https only, no credentials in the URL,
  default ports only, and never a loopback, private, link-local (cloud
  metadata), carrier-NAT, multicast or otherwise special address — checked on
  the URL, on what the host name resolves to at connect time (DNS rebinding),
  on the peer actually connected to, and again on each of at most 5 redirects.
  `TOKULY_NEWS_ALLOW_LOOPBACK=true` (refused in production) additionally
  permits loopback and non-default ports, for a local feed server. One fetch
  is bounded by `TOKULY_NEWS_FETCH_TIMEOUT_SECONDS` and
  `TOKULY_NEWS_MAX_FEED_BYTES`; nothing but the feed document is requested.
- **Articles**: markup is stripped to plain text (scripts and styles dropped,
  character references decoded once, unterminated tags swallowed); the image
  comes from the feed (MediaRSS content/thumbnail, enclosures, or the first
  `<img>` of the entry); tags are the source's default tags + the feed's
  categories + keyword matches from the dictionary, normalized (NFKC,
  lowercase) and capped at 12. The canonical URL (no fragment, no `utm_*` /
  `fbclid` / `gclid`… parameters) identifies an article: carried by two
  feeds, it is stored once and belongs to the source that brought it first.
- **Failures** (network, HTTP status, oversized, malformed, refused) are
  recorded on the source (`last_error`, `consecutive_failures`) and the next
  attempt backs off exponentially (interval × 2ⁿ, at most a day).
- **Pruning**: an article older than `TOKULY_NEWS_RETENTION_DAYS` is deleted
  once it left its feed (not seen for 12 h before the feed document was last
  read), or when its source is switched off or has not answered for 7 days.
- **Personalisation**: the first click of a reader on an article adds 1 to
  their interest in each of its tags; "more like this" adds 3, "less like
  this" subtracts 3 (changing feedback first takes back what is left of the
  earlier one). Interests halve every `TOKULY_NEWS_AFFINITY_HALF_LIFE_DAYS`
  and are kept within ±20. The recommended order ranks the 500 most recent
  articles by freshness × interest and fills `TOKULY_NEWS_RANDOM_SHARE_PERCENT`
  of the positions, evenly spaced, with random recent articles the reader has
  not opened, flagged `random_pick`. The order of a reading session is kept in
  Valkey for 30 minutes and can be rebuilt from the cursor alone.
- **Comments, likes, reports**: a `SubjectResolver` is registered for
  `SUBJECT_TYPE_NEWS_ARTICLE` (owner = nil UUID: official content, nobody is
  notified); `counter_changed("like_count" | "comment_count")` is applied.
- Analytics: `news.click` (`subject` = `news`, `props = { source_id, first }`).

## Background work

| What | Name | Notes |
|---|---|---|
| Event consumer | durable `feed` | topics above |
| News fetch job | `feed.news.fetch` (`tokuly_feed::events::FetchSourceJob { source_id }`) | one source per job; a per-source lock in Valkey prevents concurrent fetches; fetch failures are recorded, not retried by the queue |
| News scheduler | 30 s timer | claims due sources (`FOR UPDATE SKIP LOCKED`), queues their jobs; creates the sources of `TOKULY_NEWS_FEEDS` at start |
| Retention sweep | hourly timer | activities older than `TOKULY_FEED_ACTIVITY_RETENTION_DAYS`; map-story seen marks older than the story window + 24 h; handled-event markers and remembered content changes older than 8 days; news articles (see pruning); interests that faded below 0.05; recap notice markers older than 400 days |
| Recap notices | hourly timer | `feed.recap_ready` (above) |

Recaps are computed on demand and cached in Valkey (`feed:recap:…`):
`TOKULY_FEED_RECAP_CACHE_SECONDS` for a period that is still running, 6 hours
for a finished one. Deleting or clearing entries invalidates the user's
recaps at once; names and blocked people are resolved on every request.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_FEED_ACTIVITY_RETENTION_DAYS` | `400` | days an activity entry is kept; `0` keeps entries forever |
| `TOKULY_FEED_MAP_STORY_HOURS` | `48` | how far back map stories reach |
| `TOKULY_FEED_BURST_MINUTES` | `60` | entries of one person and kind no further apart than this are one card; `0` disables collapsing |
| `TOKULY_FEED_RECAP_CACHE_SECONDS` | `300` | cache time of a running period's recap |
| `TOKULY_FEED_RECAP_MIN_ACTIVITIES` | `5` | entries in a month needed for the recap notice |
| `TOKULY_NEWS_FEEDS` | empty | comma-separated RSS/Atom URLs created as sources at start |
| `TOKULY_NEWS_FETCH_INTERVAL_SECONDS` | `900` | time between two fetches of a source (≥ 60) |
| `TOKULY_NEWS_FETCH_TIMEOUT_SECONDS` | `15` | time one fetch may take, redirects included |
| `TOKULY_NEWS_MAX_FEED_BYTES` | `5242880` | largest feed document accepted |
| `TOKULY_NEWS_ALLOW_LOOPBACK` | `false` | development/tests only: let the fetcher reach loopback and non-default ports; refused when `TOKULY_ENV=production` |
| `TOKULY_NEWS_RETENTION_DAYS` | `30` | days an article is kept after publication (and after leaving its feed) |
| `TOKULY_NEWS_RANDOM_SHARE_PERCENT` | `20` | share of random picks in the recommended order (0–100) |
| `TOKULY_NEWS_AFFINITY_HALF_LIFE_DAYS` | `30` | half-life of a reader's interest in a tag |
| `TOKULY_NEWS_TAG_KEYWORDS` | empty | keyword dictionary `tag=keyword|keyword;tag=…`; empty = built-in dictionary, `-` = none |

No new infrastructure: PostgreSQL, Valkey (caches, locks, rate limits),
NATS (events, the fetch job) and outgoing HTTP(S) from worker processes to
the configured news feeds.

## Other integration points

- Personal data export: section `feed` (activities, hidden kinds, map-story
  views, news clicks / feedback / interests, recap notices).
- Rust API (`tokuly_feed`): `Feed::new(core, identity, cfg)`,
  `Feed::register(router)`, `Feed::spawn_workers()`; for workers and tests:
  `handle_event`, `refresh_source`, `enqueue_due_fetches`,
  `seed_config_sources`, `run_retention`, `send_recap_notices`, `purge_user`.
