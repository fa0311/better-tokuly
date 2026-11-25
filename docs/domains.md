# Product scope by domain

What each backend domain must provide. Sources: the original Tokuly (location
sharing + notes + stories + DM/calls + news + Tokuly Live), Bump / Zenly, and
the features requested for the rebuild (stamps, shorts, logs).

Shared API types live in `proto/tokuly/common/v1/common.proto`; conventions in
`backend/README.md`.

## Who it is for

A super app with a live map of your friends at its centre.

- **High-school students and Gen Z in Japan first.** School life, friend
  groups, 推し活, streaks, anonymous questions, study sessions, hanging out in
  voice rooms. Playful by default (the ドパガキ度 setting scales how much),
  safe by default: friends-only audiences, ghost mode, reporting on
  everything, anonymity that never shields abuse from moderators.
- **Every generation.** Nothing requires knowing the slang; families use the
  map, places and safety features without touching the rest.
- **Engineers.** Every feature is an API (Connect RPC / gRPC), with personal
  tokens, OAuth apps, bots and webhooks; custom emoji, rich text markup,
  themes, keyboard-driven screens; self-hostable with one `docker compose up`.

Where features come from: the original Tokuly (everything it had), Bump /
Zenly (map), LINE (chat, stamps), Instagram (stories, close friends, photo
grid), Twitter / X (timeline, quotes, trends, communities, audio rooms),
Misskey (custom emoji, emoji reactions, markup, antennas, channels, clips,
federation), YouTube / Twitch / TikTok (video, live, shorts), BeReal (POV),
質問箱 / Gas (anonymous questions, compliment polls), Studyplus (study
logs), 調整さん (date polls), Discord (communities, always-on voice rooms).

## identity · social · geo · realtime · media

See their proto files. Summary of behaviour that is not obvious from the API:

- **identity**: accounts, sessions (rotating refresh tokens, passkeys, TOTP,
  device login), profiles and settings. Profiles carry the **プロフ帳**:
  MBTI, blood type, zodiac (derived from the birthday), pronouns, 推し (with
  an image or emoji and a link), favourite things as tags in categories
  (music, games, food, anime…, suggested from what others show publicly),
  links verified by a `rel="me"` link back (checked in the background
  through the egress guard), a longer bio, a theme colour and style, a song
  pinned from Spotify, and question-and-answer cards from a curated bank of
  prompts ("好きな教科", "無人島に1つ持っていくなら") or free ones,
  reorderable and hideable. Every field and every card has its own audience
  (public / friends / only me); a block in either direction hides the whole
  profile; moderators can open a profile in full. The public fields are in
  the account's search document. **Birthdays** are shown per one privacy
  setting (nobody / friends / everyone, month and day or the full date;
  never the year to everyone under 18), which every domain reads.
  **Bots** (made on the developer platform) carry the bot flag and their
  owner on the account; bots never count as people (`bots::humans_only`).
  **Several accounts on one device** (main, 裏垢, creator): each is added
  by a full sign-in and keeps its own session; switching needs no password
  but asks for the target's second factor when it has not passed it there
  within 12 hours; the server never links the accounts, signing out one
  leaves the others; at most 5 per device.

- **social**: friendships are mutual; a request to someone who already
  requested you auto-accepts. Best friends are ranked automatically from
  interactions (messages, stamps, bumps, time together, map views) with a
  weekly streak counter, plus manual favourites. Blocking is silent and hides
  both users from each other everywhere. Invite links/QR count "intros".
  Nearby discovery pairs users with the add-friend screen open within ~50 m
  (minors only with minors, adults only with adults); an account signed up
  under 18 starts with nearby discovery off (existing accounts keep their
  setting). Mutual friends (on
  profiles, in search, requests and suggestions) follow the other user's
  friend list audience, like their best friends.
  A bump is two shake attempts within ~4 s and ~30 m; friends of both get
  "A and B bumped!". Strangers bump only when both allow nearby discovery and are
  in the same age group. Buzz needs the two to be within 200 m. Map reactions
  ("supersends": hearts, eyes, drinks, emoji, stamps) play live or are queued
  until the recipient opens the app.
  Follows sit next to friendships for public content: one way, with locked
  accounts (followers need approval, declining is silent; minors start
  locked), removable followers, follower / following / mutual lists and
  "followed by people you follow", counts on profiles, a per-followee mute
  (optionally timed) and post notifications (all / none), and follow
  suggestions scored from pluggable signals (follows you, friends of
  friends, followed by people you follow, popular at your school). A
  friendship does not imply a follow nor the reverse; both are audiences,
  with friends inside "followers". Circles — the built-in close friends and
  custom lists with a name, emoji and colour — take friends and followers
  only, lose members whose tie ends, and never tell members; content shared
  with close friends may carry a mark. Blocks end follows and circle
  memberships both ways. Other domains decide visibility through one
  audience API (public, followers, friends, close friends, a circle,
  private) with batched checks and fan-out lists.
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

- Notes: posts that start as a quick "今何してる?" and may grow to 3000
  characters with a small server-parsed markup (bold, italic, strike, code,
  quotes, links, mentions, hashtags, custom emoji, a safe set of animated
  effects), up to ten images with alt text and sensitive marks or one
  video, optional stamp, place tag, poll (2–10 choices, private votes) and
  content warning. Audiences: public, followers, friends, close friends, a
  circle, named people, private; locked accounts' public notes reach their
  followers only. Replies (threads), likes, renotes and quote notes (by
  audience), emoji reactions through engagement, edits within an hour with
  history, delete and redraft, server drafts and scheduled notes, up to five
  pinned notes, note analytics for the author. Timelines: friends,
  following, global, per user (media grid), per hashtag, mentions, lists,
  antennas, channels, clips; muted words, conversations and renotes; trends
  (hashtags and notes, per language, resistant to fresh-account crowds);
  post notifications for followers who ask. First-use rules agreement.
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
  Emoji are Unicode or custom (Misskey-style `:shortcode:`, approved in the
  emoji catalogue; drawn with their picture and a still for reduced motion,
  shown as text once taken down; usage feeds "recently used"/"popular").
  One reaction per person on content (Misskey), several distinct emoji per
  person on group conversation such as community threads and plans
  (Discord); a cap of distinct emoji per subject. Owners choose whether
  their content takes custom and sensitive emoji. "Who reacted" per emoji
  without blocked or muted people; reactions pop live on the subject's
  screens; the owner's notification is grouped ("Aと他2人がリアクション").
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


## emoji — カスタム絵文字

Misskey-style custom emoji: small images with a `:shortcode:` that work in
text, display names and reactions everywhere.

- An emoji has a unique name (`a-z0-9_`), an image (static or animated),
  a category, aliases for search (Japanese and English keywords), an author
  and licence note, and a flag for "sensitive". Anyone may submit one; it is
  usable once a moderator approves it (trusted submitters skip review);
  staff add, edit, disable and delete directly.
- Import and export of whole sets as a ZIP in Misskey's format (`meta.json`
  plus images), so existing emoji packs can be brought over.
- Catalogue for clients: the whole approved set with a version so clients
  cache it and fetch only changes; search by name / alias / category.
- Per user: a reaction deck (pinned emoji in order), recently used, usage
  counts for "popular" sorting.
- Public Rust API: does a name exist, resolve names to images.

## ask — 質問箱・ほめ投票

Anonymous questions and friendly anonymous polls, the way teenagers use
質問箱 / Peing / NGL and Gas.

- Question box per user: open or closed, who may ask (anyone signed in,
  friends), a prompt line, shareable link. Questions are anonymous by
  default (the asker may sign), ≤ 300 chars. The asker's identity is stored
  for moderation only and never shown to the recipient; the recipient can
  block "this sender" without learning who it is, filter words, and report.
- Inbox: new / answered / archived. An answer (text ≤ 1000, optional image
  or stamp) is published to the owner's Q&A page with an audience (public,
  friends) or kept private; the asker is told when their question is
  answered. Answers can be liked and commented through engagement.
- Prompts (お題): the owner asks their friends something ("私の第一印象は?")
  and collects anonymous or signed responses, choosing which to publish.
- Compliment polls (ほめ投票): rounds of positive questions from a curated
  bank ("一番おしゃれな人は?"), each with four of the voter's friends to
  pick from (shuffle, skip). The picked friend receives an anonymous
  "flame"; hints about the voter (first letter of the name…) cost coins.
  Rounds refill on a timer; finishing one is rewarded.

## quest — ミッション・バッジ・レベル・ストリーク

Rewards for using the app with friends. Entirely event-driven: it listens to
what the other domains emit.

- XP and levels, with per-source daily caps so nothing can be farmed.
- Daily and weekly missions drawn from a pool (send a stamp, visit a new
  place, study 30 minutes, answer a question…), progress, claimable rewards
  (coins, XP, a badge), one reroll a day.
- Badges with tiers and rarity (share of users who own it), some secret;
  a showcase on the profile; selectable titles (称号) next to the name.
- Friend streaks 🔥: consecutive days two friends interacted; a warning
  before a streak expires, milestones, one restore a month for coins.
- Leaderboards among friends (weekly XP, longest streak, explored area,
  study time), and monthly seasons with a season badge.
- Realtime `QuestEvent` (XP gained, level up, badge unlocked) so clients
  celebrate in proportion to the ドパガキ度.

## plans — ひま・あそぶ予定・日程調整・記念日

- "ひま" status: free now / today / this weekend, with a note, a time window
  and an audience (friends, best friends, chosen people); friends see who is
  free and can answer "あそぼ!" in one tap. Expires by itself.
- Plans: title, emoji, time (or "to be decided"), place, invitees, RSVP
  (going / maybe / no) with a comment, reminders, a checklist (持ち物), and
  a cost split (割り勘: who paid what, who owes whom).
- Date polls (調整さん): candidate dates or time slots, ○ △ × per
  participant, the best slot suggested, finalising turns it into a plan.
- Countdowns and anniversaries: to an exam, a festival, a concert, a
  birthday; "days since" with milestones (100 days, half a year, a year) for
  friendships and couples; shared with chosen people or pinned to the profile.
- A calendar view: my month of plans, polls, countdowns and friends' birthdays.

## school — 時間割・勉強・テスト

- Timetable (時間割): terms, period times, subjects with colour / teacher /
  room, weekly grid with A/B weeks and one-off changes; shared with chosen
  friends; copy a classmate's as a starting point; "who shares my free
  period" (空きコマ); the current / next class as a status.
- Study sessions: a focus timer (stopwatch or pomodoro) per subject, daily
  and weekly totals, a goal and streak, a heatmap calendar, "勉強中" shown to
  friends, weekly ranking among friends.
- Study rooms (もくもく部屋): a shared timer everyone in the room follows,
  presence, cheers; public or friends-only.
- Exams and assignments: exam periods with countdowns ("テスト期間" on the
  profile), assignments with due dates and reminders.

## spaces — たまり場（音声ルーム）

Drop-in audio rooms on LiveKit: Twitter Spaces for public talk, an always-on
Discord-style voice room for a friend group.

- A room has a title, tags, an audience (public, friends, invited people),
  a host, co-hosts, speakers on stage and listeners; raise a hand, invite to
  the stage, mute, remove, ban; emoji bursts; a pinned note or link;
  scheduled rooms with reminders.
- たまり場: a persistent room owned by a group of friends that is simply
  "open"; who is inside shows on friends' lists and the map.
- Live now among friends and trending public rooms; notifications when a
  friend opens one or invites you.

## community — コミュニティ

Places for a class, a club, a game, an 推し, a neighbourhood, a tech topic.

- A community has a name, handle, icon, banner, description, rules, tags and
  category; it is public, restricted (request, invite link / QR / code) or
  hidden. Roles (owner, admin, moderator, member), bans, join questions.
- Boards: channels holding threads (title, body with images / stamps / a
  poll) and replies; pin, lock, announce; hot / new / top; optional
  anonymous channels with a stable pseudonym per thread (moderators see the
  author); slow mode, word filters, auto-hide on reports.
- Events: time, place, capacity, RSVP with waitlist, reminders, check-in.
- Announcements, per-member notification level, member directory.
- Discovery: by category and tag, trending, from friends and school.

## dev — 開発者プラットフォーム

The whole API is open to the people who use it.

- Personal access tokens with scopes and expiry; OAuth 2.1 apps
  (authorization code + PKCE, device code for CLIs) with a consent screen
  and a list of authorized apps; scopes are `<domain>:read` / `<domain>:write`
  with location in a scope of its own.
- Bots: accounts owned by a person, marked as bots, acting through the API.
- Webhooks: signed deliveries of the owner's own events to a URL, retries,
  a delivery log with replay, automatic disabling of dead endpoints.
- An API reference generated from the proto files that the server itself
  serves, and per-token usage statistics.

## Planned extensions of existing domains

Collected here so the next pass over each domain has its list:

- **federation** (new, after notes): ActivityPub, so accounts and notes
  interoperate with Misskey and Mastodon.
- **web**: deck layout, command palette, theme editor, API explorer.

