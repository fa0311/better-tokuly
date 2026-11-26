# engagement — events, jobs and configuration

Crate `tokuly-engagement`, migration `1100_engagement.sql`, API
`tokuly.engagement.v1` (`CommentService`, `CommentModerationService`,
`ReactionService`, `BookmarkService`, `EngagementService`, `ReportService`).
Depends on `identity`, `media` (comment images: uploads with
`ASSET_PURPOSE_COMMENT_IMAGE`, or with `NOTE_IMAGE` / `CHAT_IMAGE` as
before that purpose existed), `stamp` (comment stamps),
`social` (blocks) and `emoji` (custom emoji reactions). No ClickHouse table of its own; no background jobs, one
timer (the purge of comment texts kept for reports).

Engagement knows no content. Everything attaches to a
`tokuly.common.v1.SubjectRef` and is authorized through `core.subjects`.

## For the domain that owns a subject type

Register a `SubjectResolver` for your `SubjectType`; that is all it takes to
get comments, likes, saves, shares and reports on your content.

- `resolve(id, viewer)` is asked on every request. It decides who sees the
  subject (`None` = not found), who owns it (`owner_id`; the nil UUID =
  nobody, e.g. official content: no notifications, no moderation rights),
  the `title` / `path` / `image_url` used in notifications, reports and list
  previews, and `interactions_enabled`: when false, comments and reactions
  are refused (`COMMENTS_DISABLED` / `INTERACTIONS_DISABLED`) and existing
  comments are listed for the owner only. Bookmarks, shares and reports do
  not depend on it. `id` may be a UUID or a short ID; engagement stores the
  UUID you return.
- `resolve_for_report(id, viewer)` (optional, defaults to `resolve`) is what
  `CreateReport` asks instead. A domain whose content can be withdrawn after
  it reached someone (an unsent chat message) answers here with what the
  viewer received, so the withdrawal does not prevent the report. Its
  `title` (e.g. every version of an edited message) goes to the moderation
  team only, as `reported_content` of `engagement.report.created`; the
  reporter's copy shows what `resolve` answers them now, or a generic label
  (「コメント」, 「メッセージ」, 「アカウント」, 「投稿」).
- `counter_changed(id, counter, delta)` is called after the change committed
  (best effort, never retried — treat it as a hint for denormalized copies;
  the exact numbers are `EngagementService.GetEngagement`):

  | `counter` | counts |
  |---|---|
  | `comment_count` | comments in public view: published top-level comments and the published replies below them (hiding a thread subtracts its replies too) |
  | `like_count`, `dislike_count` | likes / dislikes |
  | `reaction_count` | emoji reactions, all emoji together (Unicode and custom) |
  | `bookmark_count` | users who saved the subject |
  | `share_count` | recorded shares |

- On top of your resolver, engagement hides a subject from anyone in a block
  (either direction) with its owner — except for reports.
- When you delete a subject for good, say so in the same transaction with
  `tokuly_core::outbound::subject_deleted(tx, subject_type, id)` (or
  `subjects_deleted` for many): engagement then purges what it kept about
  it. Not when the subject is only hidden, and not while your
  `resolve_for_report` still answers for it — the comments on it are
  reportable for as long as it is (rules in `docs/events.md`). Until the
  event comes, lists show the subject as "no longer available".

Engagement itself registers:

- a resolver for `SUBJECT_TYPE_COMMENT` (owner = the comment's author, path
  = `<subject path>?comment=<id>`, interactions enabled while the comment is
  published), so comments can be liked and reported. Whoever had a comment
  in front of them — it is or was in public view, or they own the subject —
  can report it after it was hidden, after a block, and after it was edited
  or deleted; a comment written after a block began never reached the other
  side and is not theirs to report. For that, what an edit replaced (the
  first 10 and the latest 10 edits per comment) and what a deletion removed
  are kept for 30 days and go to moderators only, as `reported_content`.
  A takedown, the purge of the subject and the erasure of the author forget
  them at once;
- a resolver for `SUBJECT_TYPE_USER` — **only if no other domain registered
  one before `Engagement::new` ran** — resolving any non-erased account by
  UUID or handle (path `/u/<handle>`, interactions disabled), so accounts
  can be reported. A domain that registers users later replaces it;
- the guard of the realtime topic kind `subject`;
- a `DataExporter` named `engagement` (comments, the texts of them kept for
  reports, reactions, reaction settings, collections, reports, screening
  rules, blocks).

## Reactions

A reaction is a like, a dislike or an emoji: a Unicode emoji (`🎉`) or a
custom emoji of the emoji domain, stored and sent as its shortcode
(`:neko:`, lowercase). Rows: `reactions`, one per reaction, keyed by user,
subject and emoji (`''` for a like or dislike, so a person holds at most
one of the two).

**How many per person** (`ReactionRules` in every summary):

| Subjects | Per person | Why |
|---|---|---|
| everything not listed below: videos, shorts, notes, live streams, stamp packs, questions… | **one** reaction; a new one replaces the old | Misskey's rule for notes. Content made for an audience: a reaction says how the person felt, and every count is people. |
| `TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES` (default community threads, replies and events, plans) | **several distinct emoji**, up to `TOKULY_ENGAGEMENT_REACTIONS_PER_PERSON` (20); a like and a dislike replace each other | Discord's rule for messages. Group conversation: reactions are quick answers (✅ 👀 🙏), several at once are normal. |
| comments | a like or a dislike | unchanged |

A subject carries at most `TOKULY_ENGAGEMENT_REACTION_EMOJI_PER_SUBJECT`
(50) distinct emoji; once full, only emoji already on it can be added. A
full subject's summary lists every one of its emoji (`distinct_emoji` says
how many it carries), so clients offer exactly those. Both limits hold under
concurrent requests.

**Custom emoji** are checked against the emoji catalogue when added:
unknown, pending, rejected or disabled → `EMOJI_UNAVAILABLE`; flagged
`sensitive` → `SENSITIVE_EMOJI_NOT_ALLOWED` unless the subject's owner
allows them; `CUSTOM_EMOJI_NOT_ALLOWED` when the owner turned custom emoji
off (`ReactionService.UpdateReactionSettings`: custom emoji default on,
sensitive default off; content that belongs to nobody never takes sensitive
ones). An emoji that leaves the catalogue later, or is renamed, keeps its
reactions under the old shortcode: they count, can be taken back, cannot be
added again, and come back with `ReactionEmoji.available = false` (clients
show the shortcode as text).

Every response that carries reactions gives custom emoji as
`ReactionEmoji { name, image, static_url, animated, sensitive, available }`
(`static_url`: a still for reduced motion). Stored reactions are reported to
`tokuly_emoji::catalog::record_use`, so "recently used" and "popular" follow.

**Who reacted** (`ListReactions`): emoji reactions are public to everyone
who sees the subject, likes to the owner, dislikes to nobody; people in a
block with the caller and people the caller muted are left out (mutes are
mirrored from `social.friend.prefs_changed` and `notify.mute.changed`).

## Channel identities

Comments can be written as a channel. The server wires the video domain in:

```rust
engagement.set_channel_directory(Arc::new(MyChannels));   // impl tokuly_engagement::ChannelDirectory

#[async_trait]
pub trait ChannelDirectory: Send + Sync + 'static {
    async fn owned_channel(&self, user_id: Uuid, channel_id: Uuid) -> AppResult<Option<ChannelSummary>>;
    async fn summaries(&self, ids: &[Uuid]) -> AppResult<HashMap<Uuid, ChannelSummary>>;
}
```

Without a directory `CreateComment` with a `channel_id` fails with
`CHANNEL_NOT_FOUND`. A comment written as a channel never reveals its
account: `Author.user` is unset for everyone but the author, and its
notifications carry no actor (the channel's name and icon instead).

## Events emitted

Payload structs live in `tokuly_engagement::events`. All are written in the
transaction of the change. `subject_type` is `tokuly.common.v1.SubjectType`
as its number, `subject_id` the subject's UUID.

| Topic | Payload | When |
|---|---|---|
| `engagement.report.created` | `{ report_id, reporter_id, subject_type, subject_id, subject_owner_id, category, text, subject_title, subject_path, reported_content }` | A user reported a subject. `subject_title`: what the reporter may see of it now (a generic label such as 「コメント」 when it was withdrawn or is hidden from them). `reported_content`: what they received (`resolve_for_report`; for content edited or withdrawn since, every version) — for moderators only. `subject_owner_id` is `null` for content that belongs to nobody. `category` is `SPAM`, `HARASSMENT`, `HATE`, `SEXUAL`, `VIOLENCE`, `SELF_HARM`, `ILLEGAL`, `IMPERSONATION`, `COPYRIGHT` or `OTHER`. Actor: the reporter. Consumed by admin. |
| `engagement.comment.published` | `{ comment_id, subject_type, subject_id, subject_owner_id, author_id, channel_id, parent_id }` | A comment entered public view: written (and not held), or approved by the owner. Actor: the author, or the owner who approved. |
| `engagement.comment.edited` | the fields above, plus `body` (`{ ja, en }`: what notifications show for the new text), `mentioned` (users the new text mentions), `owner_notice` (top-level comments: `{ user_id, kind, title }`, what the subject owner's notification becomes when the new text no longer mentions them; `null` for replies) and `edited_at` (the comment's new version; orders two edits) | The author changed the text of a comment, whatever its status (when the edit holds it again, `engagement.comment.hidden` is emitted first). Notify updates inbox entries and queued pushes, and withdraws mentions the edit took out. Carries comment text: internal consumers only, never webhooks. Actor: the author. |
| `engagement.comment.hidden` | the fields above, plus `status` (`CommentStatus` number: 2 held, 3 hidden), `hold_reason` (`HoldReason` number), `visible_to` (who besides the author may still see it: `[subject owner]` when the owner hid it or it waits for their review, `[]` when a block hid it) and `changed_at` (orders it against the comment's other changes) | A comment left public view (or the review queue) without being deleted: hidden by the subject owner (`HideComment`), by a block (`social.user.blocked`, `BlockCommenter`), or held again by an edit that trips the owner's rules. Its replies left public view with it. Notify withdraws inbox entries about it and its replies from everyone else. Actor: the owner, the author (an edit), or nobody (a block). |
| `engagement.comment.deleted` | same | A comment was deleted by its author, the subject owner, moderation, or because its only content was an upload deleted by force. Not emitted for the replies that go with a thread, nor for comments purged with their subject or their author's account. |
| `engagement.reaction.changed` | `{ user_id, subject_type, subject_id, subject_owner_id, kind, emoji, previous_kind, previous_emoji }` | A reaction came or went: one event per reaction. `kind` / `previous_kind` are `ReactionKind` numbers (1 like, 2 dislike, 3 emoji), 0 = none; `emoji` is a Unicode emoji or `:name:` for a custom one. A replacement carries both sides; on a subject that takes several emoji per person, adding one has `previous_kind` 0 and taking one back has `kind` 0 (taking all back: one event each). `subject_owner_id` is `null` when a reaction was removed from a subject that no longer resolves. |
| `engagement.bookmark.changed` | `{ user_id, subject_type, subject_id, saved }` | A user saved a subject (`saved: true`) or removed it from everything they saved. Moves between named collections emit nothing. |
| `engagement.share.recorded` | `{ user_id, subject_type, subject_id, subject_owner_id, target }` | A share was recorded (a repeat by the same user or guest address with the same target within an hour is not). `user_id` is `null` for guests; `target` is the `ShareTarget` number. |

## Events consumed

Durable consumer `engagement` (`Engagement::handle_event`). Every handler is
idempotent.

| Topic | Reaction |
|---|---|
| `admin.content.removed` `{ subject_type, subject_id, … }` | `SUBJECT_TYPE_COMMENT`: the comment is deleted with its replies (counters, realtime and `engagement.comment.deleted` as for any deletion). Any other type: everything engagement kept about the subject is purged — comments with their uploads and likes, reactions, bookmarks, share counts, counters. Either way the open reports about the subject become `ACTION_TAKEN`, and the comment texts kept for reports (of the comment and its replies, or of every comment on the subject) are forgotten. |
| `subject.deleted` `{ subject_type, subject_id }` | The owning domain deleted the subject for good: everything engagement kept about it is purged as for a takedown and the subject's realtime topic is closed. Reports about it are **not** answered: a deletion by the owner is no decision of the moderation team, and the case keeps what was reported. Ignored for `SUBJECT_TYPE_COMMENT`. Idempotent. |
| `admin.report.resolved` `{ report_ids: [uuid], action_taken: bool }` | The named reports become `ACTION_TAKEN` / `NO_ACTION`. Emitted by the admin domain when a moderator resolves the case the reports were filed into. |
| `identity.user.deleted` `{ user_id }` | Erased: the user's comments (threads go with their replies), reactions, bookmarks and collections, reports they filed, screening rules, comment blocks (as owner and as blocked), mentions of them, the texts of their comments kept for reports. Purged: everything on the subjects they owned. Reports about them or their content stay, without the owner reference. Counters of other people's subjects follow, with callbacks. Works in batches of 200 per transaction. |
| `social.user.blocked` `{ user_id, blocked_id }` | Everything either of the two wrote on the other's subjects is hidden (`HIDDEN`, reason `BLOCKED_COMMENTER`) for everyone and leaves the owner's review queue. Unblocking restores nothing; the owner can approve the comments. An event that arrives after the block was lifted changes nothing. |
| `social.friend.prefs_changed` `{ user_id, friend_id, muted }` | Mirrored into `engagement_mutes` (the pair's state with the event's time; an older change than the one held is ignored, so out-of-order and repeated delivery end in the latest state). |
| `notify.mute.changed` `{ user_id, muted_user_id, muted }` | The same, for mutes made in the notification settings. |
| `media.asset.deleted` `{ asset_id, … }` | Safety net for forced deletions (a comment holds its image as `engagement:comment:<id>`, so owners and the janitor cannot delete it). A comment with text loses the image; a comment that was only the image is deleted. |

## Notification kinds

All through `tokuly_core::outbound::notify`, Japanese and English, inbox +
push, `subject` set to the subject, `data = { comment_id, parent_id, edited_at }`
for the comment kinds (`engagement.heart`: `{ comment_id, thread, edited_at }`,
since it is for the comment's own author, who keeps seeing their comment when
the thread above it is hidden) (`edited_at`: the version of the comment the
notification quotes, `null` before the first edit; see
`engagement.comment.edited`). Nobody is notified about their own action, across a
block, or as the owner of content that belongs to nobody.

| Kind | To | When | Notes |
|---|---|---|---|
| `engagement.comment` | subject owner | a top-level comment entered public view | actor = commenter (none for a channel identity; channel icon as image). Path `<subject path>?comment=<id>`. |
| `engagement.comment` | subject owner | a comment is waiting for review | no actor; collapse key `engagement.review:<type>:<subject>`. Not sent for comments of blocked commenters. |
| `engagement.reply` | author of the thread; author of the reply answered | a reply entered public view | |
| `engagement.mention` | users `@mentioned` | the comment entered public view, or an edit added the mention | only users who can see the subject; at most once a day per comment and user (editing a mention out and back in is not news) |
| `engagement.heart` | comment author | the owner gave the comment a heart | no actor; collapse key `engagement.heart:<comment>`; at most once a day per comment |
| `engagement.like` | subject owner (for a comment: its author) | someone liked or reacted with an emoji | grouped: one entry per subject (collapse key `engagement.like:<type>:<subject>`), naming the newest person and how many others hold a like or emoji there (「Aと他2人がリアクションしたよ! 🎉」, 「Aがいいねしたよ!」 when alone, 「他99+人」 beyond 99), leaving out people in a block with the owner and people the owner muted. `data = { kind, emoji, emoji_image_url, others, others_more }`. actor = the user; at most once a day per user and subject; never for dislikes |

One person gets at most one notification per comment (reply before mention
before comment). A held comment notifies its audience when it is approved,
once.

## Realtime

`EngagementEvent` on topic `subject:<type number>:<subject uuid>`
(`SubjectKey::topic`). The topic kind `subject` is guarded: only viewers for
whom the subject resolves (and who are not in a block with its owner) may
subscribe. A block, an unfriending, a narrowed note audience and a removal
from a community re-check the affected people's `subject:` subscriptions; a
subject that was removed or deleted (`subject.deleted`) closes its topic.
Other visibility changes are caught by the gateway's periodic re-check.

| Event | When |
|---|---|
| `comment_added` | a comment entered public view |
| `comment_updated` | a public comment was edited, hearted or lost its image; a thread's reply count changed |
| `comment_removed` | a comment left public view (deleted, hidden, held again); `removed_count` includes its replies |
| `comment_pinned` | the subject's pin moved |
| `counters_changed` | any counter of the subject changed: full totals (custom emoji drawn, no viewer state, no rules), pushed at once when nothing was pushed during the last second, otherwise once when that second is over |
| `reaction_added` | someone added a like or an emoji (with its picture), for an animation; who is not said. At most 10 per second per subject (Valkey counter `engagement:pop:<type>:<subject>:<second>`); more are only counted |

`comment_added` / `comment_updated` carry the comment without viewer state.
When its author has any block in place the comment is left out and clients
fetch it with `GetComment`, which refuses the blocked ones. Held and hidden
comments never appear on a topic.

To one user: `review_queue_changed { held_count }` to a subject owner
whenever the number of comments waiting for their review changes.

## Analytics (`core.analytics`, ClickHouse `events`)

| Event | Subject | Props |
|---|---|---|
| `engagement.comment` | `comment`, the comment ID | `subject_type`, `subject_id`, `reply`, `status` |
| `engagement.reaction` | `subject`, the subject ID | `subject_type`, `kind`, `emoji`, `custom` |
| `engagement.share` | `subject`, the subject ID | `subject_type`, `target` — the only per-share record (PostgreSQL keeps counts) |
| `engagement.report` | `subject`, the subject ID | `subject_type`, `category` |

## Background work (`Engagement::spawn_workers`, worker processes)

| What | Name | Notes |
|---|---|---|
| Event consumer | durable `engagement` | topics above; `ack_wait` 120 s (account erasure and purges of busy subjects take a while) |
| Timer, hourly | removed comment text purge | forgets `comment_removed_texts` 30 days after the edit or deletion (`Engagement::purge_removed_comment_texts`) |

No jobs: counters are exact by construction (they move in the transaction
that changes what they count).

## Valkey keys

| Key | Value | Purpose |
|---|---|---|
| `rl:engagement:…` | counters | rate limits (below) |
| `engagement:comment:dup:<user>:<type>:<subject>:<hash>` | marker, 60 s | the same comment (text and stamp, case- and width-insensitive) cannot be posted on the same subject twice in a row |
| `engagement:push:lead:<type>:<subject>` / `…:trail:…` | markers, 1 s / 3 s | throttle of `counters_changed` |
| `engagement:liked:<user>:<type>:<subject>` | marker, 24 h | one like / reaction notification per user and subject per day |
| `engagement:pop:<type>:<subject>:<unix second>` | counter, 2 s | at most 10 `reaction_added` per subject and second |
| `engagement:shared:<user or guest:key>:<type>:<subject>:<target>` | marker, 1 h | a repeated share is not counted again. A guest's key is an HMAC (key derived from `TOKULY_SECRET`) of their address, or of its /64 for IPv6; the address itself is never stored, also not in the guests' rate limit `rl:engagement:share:guest:<key>` |
| `engagement:heart:<comment>` | marker, 24 h | one heart notification per comment per day |
| `engagement:mentioned:<comment>:<user>` | marker, 24 h | one mention notification per comment and user per day |

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_ENGAGEMENT_COMMENTS_PER_MINUTE` | `10` | Comments and replies one user may write per minute. |
| `TOKULY_ENGAGEMENT_COMMENTS_PER_HOUR` | `120` | …and per hour. |
| `TOKULY_ENGAGEMENT_REPORTS_PER_HOUR` | `20` | Reports one user may file per hour. |
| `TOKULY_ENGAGEMENT_REPORT_REPEAT_DAYS` | `7` | Days after which an unanswered report is superseded by a new report of the same user on the same subject instead of blocking it (`ALREADY_REPORTED`). `0` = at once. |
| `TOKULY_ENGAGEMENT_MAX_COLLECTIONS` | `100` | Bookmark collections per user, the default one included. |
| `TOKULY_ENGAGEMENT_MAX_BOOKMARKS` | `5000` | Subjects per bookmark collection. |
| `TOKULY_ENGAGEMENT_REACTION_EMOJI_PER_SUBJECT` | `50` | Distinct emoji one subject may carry. |
| `TOKULY_ENGAGEMENT_REACTIONS_PER_PERSON` | `20` | Emoji one person may hold at once on a subject that takes several. |
| `TOKULY_ENGAGEMENT_MULTI_REACTION_TYPES` | `COMMUNITY_THREAD,COMMUNITY_REPLY,COMMUNITY_EVENT,PLAN` | Subject types (`SubjectType` names, prefix optional) that take several emoji per person; every other type takes one. Comments cannot be listed. |

Fixed rate limits: comment edits 30/min; reactions 120/min; reaction
settings changes 30/min; bookmark changes
(and collection creation) 120/min; shares 60/min per user, 30/min per client
address for guests; screening-rule changes 30/min; comment blocks 60/h.

## For the server crate

```rust
let engagement = Engagement::new(core.clone(), identity.clone(), cfg.engagement)?;  // after the content domains, so their resolvers win
engagement.set_channel_directory(Arc::new(video_channels));                          // optional
let router = engagement.register(router);
engagement.spawn_workers();                                                          // worker processes
```

`Engagement::new` must also run in gateway processes: the topic guard lives
in `core.topic_guards`.

**Reported subjects in the case view.** `Engagement::resolve_reported_subject(staff_id,
subject_type, subject_id)` resolves a reported subject the way its reporters
can still see it (a video made private, a comment hidden after the report).
`None` when nobody reported it, when it resolves for none of its reporters,
or when the staff member is its owner or one of its reporters. For the
admin domain's case view only (`ReportedSubjects` bridge); nothing else may
show its answer.

## Gaps and requests

- **Deleted content that is not announced.** What goes by a database
  cascade (the chat lines of an erased live stream, the messages of a
  deleted conversation) sends no `subject.deleted`; saves of those stay as
  "no longer available" rows. Subjects erased with their owner's account
  are purged through `identity.user.deleted`.
- **`Subjects::counter_changed` in `tokuly-core`** keeps its registry lock
  guard alive across the resolver's `.await` (a `match` on
  `self.resolvers.read().get(..).cloned()`), which makes the returned future
  `!Send`. Engagement drives the callback on a blocking thread to keep its
  handlers `Send`; binding the cloned resolver to a local before awaiting
  would remove the need.
