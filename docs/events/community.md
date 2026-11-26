# community — events, jobs and configuration

Crate `tokuly-community`, migration `2300_community.sql`, API
`tokuly.community.v1` (`CommunityService`, `CommunityBoardService`,
`CommunityModerationService`, `CommunityEventService`,
`CommunityDiscoveryService`). Depends on `identity`, `social` (friends,
blocks), `media` (icons, banners, post images, event covers) and `stamp`
(stamps in posts). No ClickHouse table of its own.

## What it is

Communities (コミュニティ) for a class, a club, a game, an 推し, a
neighbourhood, a study group, a tech topic. A community has members with
roles, boards (channels holding threads and replies), events, announcements
and its own safety tools.

- **Visibility.** Public: anyone reads and joins. Restricted: anyone sees
  the front page; joining takes the staff's approval or an invite. Hidden:
  exists only for its members; invite only.
- **Roles.** Owner, admin, moderator, member. Who may do what is one table,
  in `crates/community/src/permissions.rs`; a member only ever acts on
  members of a lower role. Every response with a `viewer` block tells the
  client what the caller may do.
- **Site moderation.** Accounts holding the site-wide `moderator` (or
  `admin`) role read every community, including hidden ones, and may look up
  the author of an anonymous post. They hold no other community permission.

## Anonymous channels

In an anonymous channel a post shows a pseudonym — "あおいペンギン #k3f9" —
instead of its author.

- The pseudonym is `HMAC-SHA256(server key, thread ID ‖ author ID)` mapped
  to an animal, a colour and a four-character tag
  (`tokuly_community::pseudonym::pseudonym`). It is **stable within a
  thread** (the author is the same name on the thread and on each reply),
  **unrelated across threads**, and **not reversible** without the server
  secret. Nothing is stored; it is recomputed on every read.
- The real author stays in the database and **no member-facing response
  returns it** — not to members, not to the staff, not to site moderation.
  The author alone is told a post is theirs (`viewer.mine`). The moderation
  log, the review queue, realtime pushes, notifications (no actor, the
  pseudonym in the text), the search document (`owner_id` unset) and the
  events on the bus (`author_id` null, no actor) all carry the pseudonym or
  nothing.
- **Unmasking is one RPC**, `CommunityModerationService.RevealAuthor`, for
  members with `REVEAL_AUTHOR` (moderators and up) and for site moderation.
  It needs a reason, writes a log entry (who looked, at which post, why — not
  the answer), and tells the rest of the staff at once (`community.author_revealed`
  notification and an `AuthorRevealed` push).
  A moderator who is part of the matter gets `CONFLICT_OF_INTEREST`:
  someone else on the staff, or site moderation, has to look. Because a
  pseudonym is one person within a thread, the rule looks at the whole
  thread: a party is whoever wrote in it themselves (thread or reply), and
  whoever is mentioned, answered or was reported against by anything that
  author wrote in it. Removed posts can still be looked up while they
  are retained.
- **To the generic domains an anonymous post belongs to nobody.** Its
  subject resolves with the nil UUID as owner (like official content):
  engagement hides a subject from anyone in a block with its owner and
  purges it with its owner's account, and either would say who wrote the
  post. So `engagement.report.created.subject_owner_id` is nil for these
  posts, and **site moderation asks this domain**:
  `Community::author_for_moderation(subject_type, subject_id)` (see
  "Gaps and requests").
- **Nothing a member can see moves with the author's account.** Leaving,
  being banned from the community, being suspended site-wide and erasing
  the account all leave the post, its pseudonym, its picture, its poll
  numbers and the log entries about it exactly as they were. (Posts hold
  their uploads, and the media domain only collects uploads nothing holds.)
- **Blocks do not filter anonymous posts**, in lists, notifications,
  pushes, or when writing: someone the author blocked still reads and
  answers the thread. Hiding or refusing would be a way to unmask: block
  someone, see what disappears. Named posts of blocked users are hidden as
  everywhere else.
- **The staff has no exemptions there.** In named channels the staff's posts
  skip slow mode, the word filter and auto-hide; in an anonymous channel
  they do not — a post that sails past the filter would be known to be the
  staff's.
- **Attachments must not name anyone.** Only stamps of published packs
  (`STAMP_NOT_ALLOWED` otherwise: a personal stamp is its maker's), only
  uploads nothing else uses (`ASSET_ALREADY_USED`: the same file under the
  author's name elsewhere would link the two). The media domain shows
  neither the uploader nor the file name of an upload to anyone else, and
  file URLs carry only the asset ID.
- **Enough members.** An anonymous channel takes posts once
  `TOKULY_COMMUNITY_ANONYMOUS_MIN_MEMBERS` members can write in it
  (`TOO_FEW_MEMBERS`; the staff alone counts where only the staff posts).
  With two members, "anonymous" is the other one.
- Mentions from behind a pseudonym cannot be blocked, so they are capped:
  one author rings one member at most 5 times an hour (200 an hour in all);
  further mentions stay in the text and notify nobody.
- A channel's mode cannot be changed once it has threads.

## Invites

An invite is its creator's word and does not outlive their standing: it
works only while the creator is a member who may still invite. Invites are
revoked when their creator leaves, is removed or banned, or is demoted
below the right to invite; turning "members can invite" off disables
members' invites until it is turned on again.

## Removed and edited content

Deleting a post, rewording it, deleting its channel or closing the whole
community is not a way to make something disappear before anyone acted on
it.

- A removed thread, reply or event **keeps its content and uploads** for
  `TOKULY_COMMUNITY_REMOVED_RETENTION_DAYS`, counted **from the removal**
  (for content of a closed community: from the closing) — never from when
  it was written. Members no longer see it.
- It **stays reportable**: the subject keeps resolving for whoever could
  read it, with a neutral title, no image and `interactions_enabled` false.
- **Edits keep history**: every rewording of a thread, a reply or an event
  (title, text, place, link) stores the version it replaced
  (`community_revisions`). A post or event can be reworded 30 times
  (`TOO_MANY_EDITS`), so the history stays bounded and the original is
  never dropped. A rename of a community records the old name, handle and
  description in the moderation log.
- The staff and site moderation read all of it through
  `CommunityModerationService.GetContentHistory` (current content, when it
  was removed, earlier versions); authors of anonymous posts stay
  pseudonyms there. A closed community has no staff: only site moderation
  reads what was in it.
- After the retention the maintenance timer (`Community::purge_removed`)
  blanks the content, releases the uploads and drops the history, votes,
  follows and report marks. Rows stay as tombstones (counters, numbering).
  A closed community is deleted altogether once its content is purged:
  memberships, bans, invites, requests and its log go with it.
- **Nobody reviews their own post**: `ReviewPost` on one's own hidden post
  is `CONFLICT_OF_INTEREST`, whatever the decision.

## Public Rust API

```rust
Community::new(core, identity, cfg) -> anyhow::Result<Community>   // registers resolvers, topic guard, profile card, exporter
community.register(router) -> connectrpc::Router
community.spawn_workers()                                           // worker processes

community.is_member(community_id, user_id) -> AppResult<bool>
community.role_of(community_id, user_id) -> AppResult<Option<tokuly_community::Role>>
// Site moderation only: who answers for a subject of this domain (author,
// organiser, owner), removed ones included while retained. The one way to
// the author of an anonymous post besides RevealAuthor.
community.author_for_moderation(subject_type, subject_id) -> AppResult<Option<Uuid>>
// Site moderation only: is `user_id` involved in this thread or reply (the
// question RevealAuthor asks), and so not to be told who wrote it?
community.is_party_for_moderation(subject_type, subject_id, user_id) -> AppResult<bool>

tokuly_community::pseudonym::pseudonym(&core.keys, thread_id, author_id) -> tokuly_proto::community::Pseudonym
tokuly_community::permissions::{allows, permissions_of, may_assign, Role, MemberGrants}
tokuly_community::events::*                                         // payload structs of the events below

// Worker entry points (also what tests call):
community.handle_event(event) -> AppResult<()>
community.run_announcement(thread_id) -> AppResult<()>
community.run_sweep(SweepJob { community_id, channel_id }) -> AppResult<()>
community.refresh_hot_scores() -> AppResult<u64>
community.refresh_trend_scores() -> AppResult<u64>
community.send_due_reminders() -> AppResult<usize>
community.run_maintenance() -> AppResult<()>
community.purge_removed() -> AppResult<u64>                         // part of maintenance
```

## Events emitted

Payload structs live in `tokuly_community::events`. All are written in the
transaction of the change.

| Topic | Payload | When |
|---|---|---|
| `community.created` | `{ community_id, owner_id, visibility, category }` | A community was created. Actor: the creator. |
| `community.updated` | same | Its profile, visibility or owner changed. Actor: who changed it (the new owner after a transfer). |
| `community.deleted` | same | It was closed: by its owner, by a site takedown, or because its owner's account was erased and nobody was left. |
| `community.member.joined` | `{ community_id, user_id, public }` | Someone became a member (joined, invited, approved, or created the community). Actor: the member. |
| `community.member.left` | `{ community_id, user_id, public, cause }` | `cause` is `left`, `removed`, `banned` or `account_deleted`. Actor: the member. |
| `community.thread.created` | `{ community_id, channel_id, thread_id, reply_id: null, author_id, anonymous, public }` | A thread entered public view: posted, or restored after having been held by the word filter. |
| `community.reply.created` | same, with `reply_id` | A reply entered public view. |
| `community.thread.deleted` | `{ community_id, thread_id }` | A thread was deleted (author, staff, review, site takedown). Not emitted for threads that go with a deleted channel or a closed community. |
| `community.event.created` | `{ community_id, event_id, organizer_id, public }` | An event was put on the calendar. |
| `community.event.attended` | `{ community_id, event_id, user_id, public }` | A member checked in at an event (code or by hand), once per member and event. |
| `subject.deleted` | `{ subject_type, subject_id }` | Shared event of `tokuly_core::outbound::subject_deleted`, no actor. A thread, reply or event was purged (`SUBJECT_TYPE_COMMUNITY_THREAD` / `_REPLY` / `_EVENT`), or a closed community was (`SUBJECT_TYPE_COMMUNITY`): by `purge_removed`, after the retention. Removed posts answer reports until then, with the comments and reactions on them, so their removal itself is not announced this way. |

`visibility` / `category` are the enum numbers of `tokuly.community.v1`;
`public` says the community is public. **For posts in anonymous channels
`author_id` is `null` and the event has no actor**, so nothing downstream
(activity feeds, webhooks, quests) can name the author by accident.

Through `tokuly_core::outbound`:

- **Search** (`search.changed`): `kind: "community"` for communities that
  are not hidden (title = name, text = description, tags, `owner_id`, score =
  member count, `extra = { handle, category, visibility, member_count,
  school_name }`); `kind: "thread"` for visible threads of **public**
  communities (title, text = body, tags, `owner_id` unset for anonymous
  threads, score = likes + replies, `extra = { community_id,
  community_handle, community_name, channel_id, anonymous, reply_count }`).
  A change of visibility re-indexes or removes the threads (job
  `community.sweep`).
- **Activity** (`activity.recorded`): `community.created`,
  `community.joined` (private: the membership is not shown on the profile at
  that point), `community.event_attended` (private, with the event's
  location when it has one).

## Events consumed

Durable consumer `community` (`Community::handle_event`). Every handler is
idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` `{ user_id }` | Posts stay (rendered as "deleted user"; anonymous ones keep their pseudonym). Gone: memberships (member counts follow), answers to events that are not over (places go to the waitlist), join requests, follows, read markers, bans, poll votes. **Numbers others watch do not move**: polls keep their counts, and reports the account filed still count but no longer name it. Invites they created are revoked; log entries lose the reference, except entries about an anonymous post, which keep the key its pseudonym is made from. **A community they owned passes to the longest-serving admin — failing that the longest-serving moderator, then member — and is closed when nobody is left.** |
| `engagement.report.created` `{ report_id, reporter_id, subject_type, subject_id, … }` | For `COMMUNITY_THREAD` / `COMMUNITY_REPLY`: counted once per reporting **member who joined before the post was written** (fresh accounts cannot gang up on a post). At the community's threshold (or `TOKULY_COMMUNITY_AUTO_HIDE_REPORTS`) the post is hidden pending review; reports on one post take turns, so it is hidden exactly once. Not counted: outsiders, the author. Never hidden: named posts of the staff, posts the staff already restored. |
| `admin.content.removed` `{ subject_type, subject_id, reason, moderator_id }` | `COMMUNITY` closes the community; `COMMUNITY_THREAD`, `COMMUNITY_REPLY`, `COMMUNITY_EVENT` remove the item and write `CONTENT_TAKEN_DOWN` into the community's log. What was removed is retained like anything removed. The owner is not notified here (admin does). |
| `media.asset.deleted` `{ asset_id, … }` | Safety net for forced deletions (everything shown holds its upload). Icons, banners and covers are cleared; the image leaves posts; a reply that was only the image is deleted. |

## Notification kinds

All through `tokuly_core::outbound::notify`, Japanese and English, inbox +
push. Nobody is notified about their own action.

| Kind | To | When | Notes |
|---|---|---|---|
| `community.announcement` | members at level ALL or MENTIONS, the author aside | a thread became an announcement for the first time | sent by job in chunks; collapse key `community.announcement:<thread>` |
| `community.mention` | mentioned **members** at level ALL or MENTIONS | a visible post mentions them, or an edit adds the mention | only members can be mentioned |
| `community.reply` | followers of the thread at level ALL | a reply entered public view | one notice per follower until they read the thread again; the author of the reply answered is always told; collapse key `community.reply:<thread>`; a mention replaces it |
| `community.event_reminder` | going and interested, level not NONE | about a day and about an hour before the start | `data.reminder` is `day` / `hour` |
| `community.event_update` | everyone who answered (not "not going") | promoted from the waitlist, cancelled, rescheduled | `data.update` is `promoted` / `cancelled` / `rescheduled` |
| `community.join_request` | the staff | someone asks to join | collapse key `community.join_request:<community>` |
| `community.join_approved` | the requester | their request was approved | declining is silent |
| `community.membership` | the member | role changed, ownership received, removed, banned, timed out | `data.change` is `role` / `ownership` / `removed` / `banned` / `timed_out` |
| `community.moderation` | the author of a post | held for review, restored, removed by the staff | `data.change` is `held` / `restored` / `removed`; sent whatever the member's level |
| `community.review` | the staff | a post was hidden and waits for review | collapse key `community.review:<community>` |
| `community.author_revealed` | the rest of the staff | someone looked up the author of an anonymous post | names who looked and why, never the author |

Notifications about posts in anonymous channels carry the pseudonym in
their text and **no actor**. Nobody is notified across a block, except in
anonymous threads (see above).

Notification levels: ALL = everything above; MENTIONS = mentions and
announcements (plus reminders of events they answered); NONE = nothing of
that. Three kinds are sent whatever the level, because they are about the
member themselves: `community.membership`, `community.moderation` and
`community.event_update` (an event they answered was cancelled or moved, or
their place came up).

## Realtime

`CommunityEvent` on topic `community:<community uuid>`. The topic kind
`community` is guarded: anyone for a public community, members (and site
moderation) otherwise. Subscriptions end when access does
(`ServerFrame.unsubscribed`, see `docs/events/realtime.md`): every
membership change (removal, ban, leaving, a timeout or role change)
re-checks that person's subscription, and their `subject:` subscriptions
(comments and counters of threads, replies and events); a community that stops being public
re-checks every subscriber; a closed community closes the topic after
`community_changed { deleted }`.

| Event | When |
|---|---|
| `thread_created` | a thread entered public view |
| `thread_updated` | edited, pinned, locked, announcement, poll changed |
| `thread_removed` | deleted, hidden or taken down |
| `reply_created` / `reply_updated` / `reply_removed` | the same for replies, with the thread's reply count |
| `thread_counts_changed` | likes / comments moved: at once, then at most about once a second per thread |
| `channels_changed` | the channel list changed: fetch it again |
| `calendar_event_changed` | an event was created, edited, answered, cancelled or removed (`event` unset) |
| `community_changed` | profile, member count, or closed |

Posts on the topic carry no viewer state. **In a community that is not
public the topic carries IDs only** — no thread, reply or event content:
someone kicked or banned loses the subscription at once (every
membership change re-checks theirs, and a community that stops being
public re-checks every subscriber), but a push already on its way can
still reach them. Clients fetch by ID, and the fetch checks access. In a public community a
named post whose author has a block with anyone also travels as IDs only
(the fetch applies the caller's blocks). Hidden posts never appear on the
topic.

To single users: `membership_changed` (their own standing),
`staff_queue_changed` and `author_revealed` (staff).

## Subjects

Resolvers for `SUBJECT_TYPE_COMMUNITY` (owner: the community's owner; ID or
handle), `COMMUNITY_THREAD` (owner: the author; UUID or short ID),
`COMMUNITY_REPLY` (owner: the author) and `COMMUNITY_EVENT` (owner: the
organiser). **The owner of a post in an anonymous channel is the nil
UUID.** Visibility follows the community's. `interactions_enabled` is
false for locked or hidden threads and their replies, cancelled events and
removed content (which keeps resolving while it is retained, see above).
**Comments and reactions on threads and replies are for those who could
write there**: members who are not timed out, of a role the channel's
posting policy admits — so an engagement comment is not a way around
"members only", a ban, a timeout or a moderators-only channel. Events take
them from members who are not timed out. (The flag covers comments and
reactions together; see "Gaps and requests".) `counter_changed` keeps
`like_count` and `comment_count` on threads, replies and events (the hot
score of a thread moves with them).

## Analytics (`core.analytics`, ClickHouse `events`)

| Event | Subject | Props |
|---|---|---|
| `community.thread` | `community_thread`, the thread ID | `community_id`, `anonymous`, `poll`, `images`, `held` |
| `community.reply` | `community_reply`, the reply ID | `community_id`, `thread_id`, `anonymous`, `images`, `held` |

No user is attached to events about anonymous posts.

## Background work (`Community::spawn_workers`, worker processes)

| What | Name | Notes |
|---|---|---|
| Event consumer | durable `community` | topics above; `ack_wait` 120 s |
| Announcement fan-out | job `community.announce` (`AnnounceJob { thread_id }`) | 500 members per transaction; the cursor is stored on the thread, so retries and duplicate jobs continue instead of repeating |
| Sweep | job `community.sweep` (`SweepJob { community_id, channel_id }`) | after a community or channel was deleted, or a community changed visibility: the search documents of its threads; runs in-process when queueing fails |
| Hot scores | timer, `TOKULY_COMMUNITY_HOT_INTERVAL_SECONDS` | `community_hot_score()` for threads active in the last 30 days, 500 threads per statement; older ones drop to 0 |
| Trend scores | timer, `TOKULY_COMMUNITY_TREND_INTERVAL_SECONDS` | (3 × joins + 2 × threads + replies of the last 7 days) / (members + 10)^0.3 |
| Event reminders | timer, every 60 s | safe on several workers (`FOR UPDATE SKIP LOCKED`) |
| Maintenance | timer, every 10 min | deletes expired bans and declined join requests older than 7 days; purges removed content whose retention is over (200 per kind and pass); re-queues announcements whose job got lost |

No handler or worker asks the pool for a second connection while it holds
a transaction: what a transaction needs from elsewhere is read before it
opens (`tests/concurrency.rs` runs bursts against a five-connection pool).

## Valkey keys

| Key | Value | Purpose |
|---|---|---|
| `rl:community:…` | counters | rate limits (below) |
| `community:slow:<channel>:<user>` | marker, the channel's slow-mode seconds | slow mode; a product rule, enforced even when request limits are off |
| `community:push:lead:<thread>` / `…:trail:…` | markers, 1 s / 3 s | throttle of `thread_counts_changed` |
| `community:tags:<category>` | JSON, 5 min | popular tags |

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_COMMUNITY_MAX_OWNED` | `5` | Communities one account may own (the creation limit). |
| `TOKULY_COMMUNITY_MAX_JOINED` | `200` | Communities one account may be a member of. |
| `TOKULY_COMMUNITY_MAX_MEMBERS` | `5000` | Members one community may have (the member cap). |
| `TOKULY_COMMUNITY_MAX_CHANNELS` | `50` | Channels one community may have. |
| `TOKULY_COMMUNITY_AUTO_HIDE_REPORTS` | `3` | Reports by different members that hide a post until reviewed (2–50); a community may set its own. |
| `TOKULY_COMMUNITY_POSTS_PER_MINUTE` | `10` | Threads and replies one member may write per minute. |
| `TOKULY_COMMUNITY_POSTS_PER_HOUR` | `120` | Threads and replies one member may write per hour. |
| `TOKULY_COMMUNITY_ANONYMOUS_MIN_MEMBERS` | `5` | Members who must be able to post in an anonymous channel before it takes posts (at least 2). |
| `TOKULY_COMMUNITY_REMOVED_RETENTION_DAYS` | `30` | Days removed threads, replies and events keep their content, uploads and edit history, counted from the removal. |
| `TOKULY_COMMUNITY_HOT_INTERVAL_SECONDS` | `300` | Seconds between passes of the hot-score worker. |
| `TOKULY_COMMUNITY_TREND_INTERVAL_SECONDS` | `900` | Seconds between passes of the trend-score worker. |
| `TOKULY_COMMUNITY_CHECK_IN_CODE_SECONDS` | `60` | Seconds an event check-in code is shown before it changes (15–600); the previous code is accepted one period longer. |

Fixed rate limits, per account unless said otherwise: creating communities
5/h; editing a community 30/h, changing its visibility 4/day per community
(each change walks its threads); handing it over 10/day; joining 30/h, join
requests 3/day per community (each rings the staff); invites 30/h; role
changes 60/h; kicks, bans and timeouts 120/h; other staff actions (pin,
lock, channels, safety settings, reviews, deleting others' posts) 240/h; announcements 10/h per
community; author lookups 30/h; edits of posts 30/min; votes 60/min;
events 20/h, edits/cancellations of events 30/h, answers to events 30/min;
check-in tries 10/min per member and event; member search 120/min; handle
checks 60/min; invite previews 30/min; the discovery lists 120/min together
(the last three per account, or per client address for guests).

## For the server crate

```rust
let community = Community::new(core.clone(), identity.clone(), cfg.community)?;  // before Engagement::new, so its resolvers are in place
let router = community.register(router);
community.spawn_workers();                                                       // worker processes
```

`Community::new` must also run in gateway processes: the topic guard lives
in `core.topic_guards`. Add `cfg.community: CommunityConfig` (flattened) and
migration `2300` to the server.

## Gaps and requests

- **Search kinds, notify category: done.** Search has the kinds `community`
  and `thread`; notify files `community.*` under its `community` category.
- **Upload purposes.** There is no `ASSET_PURPOSE_COMMUNITY_*`. Icons accept
  `AVATAR` / `CHANNEL_ICON`, banners `BANNER` / `CHANNEL_BANNER`, post
  images `NOTE_IMAGE` / `CHAT_IMAGE`, event covers additionally
  `THUMBNAIL`. Stamps in posts are recorded as `SendContext::Note`.
- **Reports about anonymous posts (admin, server): done.** These subjects
  resolve without an owner, so `engagement.report.created.subject_owner_id`
  is nil for them. The server bridges `Community::author_for_moderation` and
  `Community::is_party_for_moderation` to the admin domain
  (`tokuly_admin::HiddenOwners`, `bridges.rs`). The case in the moderation
  queue can then warn, suspend or ban the author, under the rule of
  `RevealAuthor`: a site moderator who is involved in the post (reported it,
  wrote in the thread, is mentioned or answered by its author) is not shown
  the author and cannot act on the case, and every time the author is shown
  to another moderator the admin audit log records it
  (`docs/events/admin.md`). `RevealAuthor` (logged and announced to the
  community's staff) stays the way for a community's own staff.
  `GetContentHistory` is the matching read for removed or edited content
  named in a report.
- **Reusing an upload after posting anonymously.** This domain refuses an
  upload that is already used elsewhere; it cannot stop the author from
  attaching the same upload to a named post in another domain afterwards.
  Clients must upload afresh for anonymous posts and never offer those
  uploads again.
- **Restored posts.** A post held by the word filter and later restored
  notifies nobody but its author (no mention or follower notifications are
  sent in hindsight).
- **Comments under threads (core, engagement).** `SubjectInfo` has one
  switch for comments and reactions. This domain opens it only to members
  who could write in the channel, which covers membership, bans, timeouts
  and the posting policy — but the channel's word filter and slow mode
  cannot reach text the engagement domain stores. With a separate
  `comments_enabled` on `SubjectInfo`, threads would turn comments off
  altogether (replies are the place to answer) and keep likes for every
  reader.
