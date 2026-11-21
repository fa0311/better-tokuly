# Product scope by domain

What each backend domain must provide. Sources: the original Tokuly (location
sharing + notes + stories + DM/calls + news + Tokuly Live), Bump / Zenly, and
the features requested for the rebuild (stamps, shorts, logs).

Shared API types live in `proto/tokuly/common/v1/common.proto`; conventions in
`backend/README.md`.

## identity (done) · social · geo · realtime · media — contracts written

See their proto files. Summary of behaviour that is not obvious from the API:

- **social**: friendships are mutual; a request to someone who already
  requested you auto-accepts. Best friends are ranked automatically from
  interactions (messages, stamps, bumps, time together, map views) with a
  weekly streak counter, plus manual favourites. Blocking is silent and hides
  both users from each other everywhere. Invite links/QR count "intros".
  Nearby discovery pairs users with the add-friend screen open within ~50 m.
  A bump is two shake attempts within ~4 s and ~30 m; friends of both get
  "A and B bumped!". Buzz needs the two to be within 200 m. Map reactions
  ("supersends": hearts, eyes, drinks, emoji, stamps) play live or are queued
  until the recipient opens the app.
- **geo**: location samples arrive over the realtime gateway or
  `ReportLocations`. Latest state per user is kept in Valkey and fanned out to
  the friends allowed to see it, honouring ghost mode (precise / blurred /
  frozen, per friend, optionally timed). The pipeline derives stays → visits
  (matched to places; home/work/school auto-detected), nights, gatherings
  (friends within 30 m: "N人が集合中!"), footprints (H3 cells; % explored per
  country / prefecture), trips with ETA, place alerts, SOS. Raw history goes to
  ClickHouse for day timelines and route replay.
- **realtime**: WebTransport + WebSocket gateway speaking
  `tokuly.realtime.v1` frames; forwards frames published by any domain.
- **media**: resumable uploads to object storage, image variants + thumbhash,
  video transcoding to HLS (CMAF) with poster, storyboard and preview, audio
  waveforms, link previews.

## stamp — スタンプ

Stickers usable everywhere (chat, live chat, comments, notes, map reactions).

- Packs: official packs and user-created packs. A pack has a name,
  description, cover, tags, 1–120 stamps, a creator, a price in coins (0 =
  free) and a review status (draft → in review → published / rejected; trusted
  creators auto-publish). Formats: static (WebP/PNG), animated (animated
  WebP/APNG), Lottie (dotLottie).
- Creator studio: create/edit packs, add stamps from uploaded assets
  (`ASSET_PURPOSE_STAMP`; cut-out happens on the client), reorder, rename,
  submit, unpublish, per-pack stats (installs, sends).
- Store: featured, new, popular (ranked by installs/sends), search, by
  creator, pack detail with previews.
- Library: install/uninstall, reorder installed packs, favourites, recently
  used, "save a stamp I received" (adds its pack or a personal collection).
- Quick stamps: any single image a user saves becomes a personal stamp without
  a pack (Bump-style custom stickers).
- Usage log: every send is recorded (who, where: chat/live/comment/note/map)
  for recents, rankings and creator stats.
- Public Rust API for other domains: resolve stamp IDs to `StampRef`, check a
  user may use a stamp (free/official/installed/owned/personal), record a send.
- Paid packs are bought with coins through the wallet domain.

## wallet — コイン

Virtual currency for gifts and paid stamp packs. No real payment processor is
bundled: purchases go through a pluggable provider interface, with a built-in
"development" provider that grants coins.

- Double-entry ledger; balances can never go negative; every movement is
  idempotent on a caller-supplied key.
- Accounts per user: spendable coins and creator earnings (received gifts,
  stamp sales) kept apart.
- RPCs: balance, ledger history, coin packages on sale, purchase (provider
  flow), daily login bonus, earnings summary, payout requests (reviewed by
  admins).
- Public Rust API for other domains: `spend`, `transfer` (payer → recipient
  earnings, with platform fee percentage from config), `refund`.
- Realtime `WalletEvent` on balance changes.

## chat — DM・グループ・通話

- Conversations: 1:1 (unique per pair) and groups (name, icon, up to 100
  members, owner/admin roles, invite by friend, leave, remove). Who may DM is
  governed by the recipient's privacy audience and blocks.
- Messages: text (with mentions and link previews; YouTube/Spotify links embed
  inline), stamps, images/video/voice/files (media attachments), location
  shares, replies, forwards, big-emoji, system messages. Edit, delete for
  everyone, pin, emoji reactions, read receipts, typing indicators, unread
  counts, mute, hide/archive, per-conversation search.
- Live co-presence: who has the conversation open right now.
- POV requests (Bump): ask a friend for a live photo/video of where they are;
  2 h to answer, one open request per conversation, reply expires after 24 h
  unless saved.
- "First hello" shortcut and message requests from non-friends when allowed.
- Calls: 1:1 and group voice/video through LiveKit — ring, accept, decline,
  join, leave, missed-call records; the service mints LiveKit access tokens.
- AI assist (optional, when an Anthropic API key is configured): rewrite a
  draft in another tone, suggest replies, translate a message.
- Realtime `ChatEvent`; push notifications collapsed per conversation.

## notes — ノート・ストーリー（記憶）

- Notes: short posts (≤ 300 chars, "今何してる?"), up to 4 images or one
  video, optional stamp, optional place tag, visibility public / friends /
  private, replies (threads), likes, renotes and quote notes, delete, hashtags
  and mentions, pin to profile. Timelines: friends, global, per user, per
  hashtag, replies thread. First-use rules agreement.
- Stories ("記憶"): title (≤ 140) plus 1–10 images/videos, visible to friends
  for 24 h, then kept in the owner's archive (browsable by day); viewers list,
  reactions, replies go to DM; story highlights on the profile.
- Like/renote counters update live for notes on screen (`NotesEvent`).

## video — 動画

YouTube-like long-form video.

- Channels: an account owns up to 5 (name, handle, icon, banner, description,
  links); subscribe with notification level; channel page with videos,
  shorts, live, playlists, about.
- Videos: created from an uploaded asset; title, description, tags, category
  /genre and game, custom or auto thumbnail, chapters, visibility (public /
  hidden from feeds / unlisted / friends / private), scheduled publishing,
  age restriction, comments on/off, like/dislike, view counting with
  de-duplication, resume position, `?t=` deep links.
- Subtitles: upload WebVTT/SRT per language, manage, default track.
- Playlists (public/unlisted/private, reorder), watch later, liked videos,
  watch history (pause/clear), subscriptions feed, home recommendations,
  trending, related videos.
- Watch parties: host-controlled synchronized playback with party chat,
  up to 100 people, guests by link.
- Creator analytics (views, watch time, retention, subscribers, traffic).
- Realtime `VideoEvent` (watch party state, processing done).

## live — ライブ配信

- Streams: title, description, genre/game, thumbnail, visibility incl.
  friends-only, scheduled start with reminders, low-latency toggle, DVR
  (rewind up to 1 h), auto-archive to a video.
- Ingest: RTMP / SRT / WHIP through MediaMTX with a public stream name and a
  secret key (rotate anytime); MediaMTX calls our auth hook. Browser
  publishing (WHIP) and playback URLs (LL-HLS, WebRTC/WHEP).
- Lifecycle driven by ingest hooks: live → ended → archived; viewer counts
  (concurrent + peak), likes.
- Live chat: guests read, members post; stamps; pinned message; slow mode,
  members-only, banned words, timeouts/bans, moderators, delete; chat replay
  synced to archive playback.
- Gifts (super chat): 12 colour tiers by coin amount with a highlighted
  message whose max length grows with the tier; spends coins via wallet,
  credits the streamer's earnings; gift history for sender and streamer.
- Polls, viewer list for the streamer, raid/host another stream at the end.
- Clips: 0.1–60 s ranges of a live archive or video, title ≤ 64.
- OBS overlays: chat and alert widgets with style presets and custom CSS.
- Realtime and per-stream analytics for the streamer.

## shorts — ショート

Vertical videos up to 180 s.

- Create from an upload or from a clip; caption with hashtags and mentions,
  cover frame, visibility, allow comments, optional sound attribution.
- Feeds: for-you (ranked), following, friends; cursor-based with
  seen-tracking so items do not repeat; "not interested".
- Likes, saves, shares, view/loop/watch-through signals feeding ranking,
  hashtag pages, sound pages, creator profile grid.
- Creator analytics.

## engagement — コメント・リアクション・通報

Generic over `SubjectRef` (uses `core.subjects`).

- Comments: threaded (one level of replies), text ≤ 1000 chars with mentions,
  optional stamp or image, edit, delete (by author or subject owner), pin by
  owner, creator heart, like/dislike on comments, sort top/new, comment as a
  channel identity, hold-for-review and blocked words per subject owner.
- Reactions: like/dislike and emoji reactions on any subject, with counts.
- Bookmarks/saves with collections, share tracking, view counters.
- Reports: reason categories + text against any subject; feeds the admin queue.
- Realtime `EngagementEvent` on `subject:{type}:{id}` topics.

## notify — 通知

- Consumes `notify.requested`; stores an inbox entry (grouped by collapse
  key) and delivers push: Web Push (VAPID), FCM, APNs (the latter two when
  credentials are configured).
- Inbox: list, unread count, mark read/all read, delete.
- Device push registrations per session.
- Preferences: per kind category on/off for inbox and push, quiet hours,
  per-friend mute comes from social.
- Realtime `NotifyEvent` (new notification, unread count).

## feed — ログ・マップストーリー・ニュース

- Activity log: consumes `activity.recorded`; a user's own log (everything),
  friends' feed (audience-filtered), profile activity list.
- Map stories: recent friend activities with a location, auto-grouped per
  friend into story reels for the map (unlock at 8 friends in Bump; no gate
  here), with seen state.
- Home feed: mixed timeline of friends' notes, stories, lives and videos.
- News: RSS/Atom sources fetched on a schedule, tagged, ranked per user from
  click history with a share of random picks (flagged as such); click
  tracking; plus product announcements (the update log).
- Yearly/monthly recap numbers.

## search — 検索

- Consumes `search.changed` into Meilisearch indexes per kind (Japanese
  tokenization), with reindex jobs.
- Unified search with per-kind tabs, suggestions/typeahead, trending queries,
  per-user recent searches. Blocked users' content is filtered out.

## admin — 管理・モデレーション

Role-gated (`admin`, `moderator`).

- Reports queue (from engagement): list, assign, resolve with an action
  (dismiss, remove content, warn, suspend, ban); moderation log.
- Users: search, view, suspend/ban/unban, grant roles, verify badge.
- Content takedown by subject; stamp pack review; payout review.
- Feature flags CRUD; announcements; server stats (users, DAU, content
  counts, queue depths); audit log of admin actions.
