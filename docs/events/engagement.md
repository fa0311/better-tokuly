# engagement — events, jobs and configuration

Crate `tokuly-engagement`, migration `1100_engagement.sql`, API
`tokuly.engagement.v1` (`CommentService`, `CommentModerationService`,
`ReactionService`, `BookmarkService`, `EngagementService`, `ReportService`).
Depends on `identity`, `media` (comment images), `stamp` (comment stamps) and
`social` (blocks). No ClickHouse table of its own; no background jobs, one
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
  team only, as `reported_content` of `engagement.report.created`. The
  reporter's copy of the report, and `subject_title`, is what `resolve`
  answers them now — or a generic label (「コメント」, 「メッセージ」,
  「アカウント」, 「投稿」) when it no longer resolves for them. Nothing else
  ever sees the `resolve_for_report` answer.
- `counter_changed(id, counter, delta)` is called after the change committed
  (best effort, never retried — treat it as a hint for denormalized copies;
  the exact numbers are `EngagementService.GetEngagement`):

  | `counter` | counts |
  |---|---|
  | `comment_count` | comments in public view: published top-level comments and the published replies below them (hiding a thread subtracts its replies too) |
  | `like_count`, `dislike_count` | likes / dislikes |
  | `reaction_count` | emoji reactions, all emoji together |
  | `bookmark_count` | users who saved the subject |
  | `share_count` | recorded shares |

- On top of your resolver, engagement hides a subject from anyone in a block
  (either direction) with its owner — except for reports.
- When you delete content, engagement is not told: its rows stay and lists
  show the subject as "no longer available". They are purged when the
  content is taken down by moderation (`admin.content.removed`) or its
  owner's account is erased. See "Gaps".

Engagement itself registers:

- a resolver for `SUBJECT_TYPE_COMMENT` (owner = the comment's author, path
  = `<subject path>?comment=<id>`, interactions enabled while the comment is
  published), so comments can be liked and reported. For a report it asks
  the subject's domain for `resolve_for_report`, and whoever had the comment
  in front of them — it is or was in public view (a reply only below a
  published top-level comment), or they own the subject — can report it
  after it was hidden, after a block, and after its author (or the
  subject's owner) edited or deleted it. A comment written after a block
  began, with its author or with the subject's owner, never reached the
  other side and is not theirs to report (`tokuly_social::graph::blocked_since`).
  What an edit replaced (the first 10 and the latest 10 edits per comment;
  each further edit drops the oldest of the latest ones) and what a
  deletion removed (the comment and the replies that went with it) are
  kept in `comment_removed_texts` for 30 days from the change; the
  moderators' `reported_content` lists every distinct version, oldest first
  (`[2版] before → after`, 80 characters each, a note where edits were not
  kept). Nothing else reads them. A takedown, the purge of the subject and
  the erasure of the author forget them at once;
- a resolver for `SUBJECT_TYPE_USER` — **only if no other domain registered
  one before `Engagement::new` ran** — resolving any non-erased account by
  UUID or handle (path `/u/<handle>`, interactions disabled), so accounts
  can be reported. A domain that registers users later replaces it;
- the guard of the realtime topic kind `subject`;
- a `DataExporter` named `engagement` (comments, the texts of them kept for
  reports, reactions, collections, reports, screening rules, blocks).

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
| `engagement.report.created` | `{ report_id, reporter_id, subject_type, subject_id, subject_owner_id, category, text, subject_title, subject_path, reported_content }` | A user reported a subject. `subject_title` is what the reporter may see of it now (what their own copy shows; a generic label such as 「コメント」 when it was withdrawn or is hidden from them). `reported_content` is what they received as the owning domain kept it (`resolve_for_report`): for content edited or withdrawn since, every version. It is for moderators only: admin keeps it on the case (`moderation_cases.reported_content`, cleared with the case's title) and shows it on the case screen. `subject_owner_id` is `null` for content that belongs to nobody. `category` is `SPAM`, `HARASSMENT`, `HATE`, `SEXUAL`, `VIOLENCE`, `SELF_HARM`, `ILLEGAL`, `IMPERSONATION`, `COPYRIGHT` or `OTHER`. Actor: the reporter. Consumed by admin. |
| `engagement.comment.published` | `{ comment_id, subject_type, subject_id, subject_owner_id, author_id, channel_id, parent_id }` | A comment entered public view: written (and not held), or approved by the owner. Actor: the author, or the owner who approved. |
| `engagement.comment.edited` | same | The author changed the text of a comment, whatever its status (an edit can also hold it again, see `comment_removed`). For copies of the old text, e.g. inbox entries. Actor: the author. |
| `engagement.comment.deleted` | same | A comment was deleted by its author, the subject owner, moderation, or because its only content was an upload deleted by force. Not emitted for the replies that go with a thread, nor for comments purged with their subject or their author's account. |
| `engagement.reaction.changed` | `{ user_id, subject_type, subject_id, subject_owner_id, kind, emoji, previous_kind, previous_emoji }` | A user set, changed or removed a reaction. `kind` / `previous_kind` are `ReactionKind` numbers (1 like, 2 dislike, 3 emoji), 0 = none. `subject_owner_id` is `null` when a reaction was removed from a subject that no longer resolves. |
| `engagement.bookmark.changed` | `{ user_id, subject_type, subject_id, saved }` | A user saved a subject (`saved: true`) or removed it from everything they saved. Moves between named collections emit nothing. |
| `engagement.share.recorded` | `{ user_id, subject_type, subject_id, subject_owner_id, target }` | A share was recorded (a repeat by the same user or guest address with the same target within an hour is not). `user_id` is `null` for guests; `target` is the `ShareTarget` number. |

## Events consumed

Durable consumer `engagement` (`Engagement::handle_event`). Every handler is
idempotent.

| Topic | Reaction |
|---|---|
| `admin.content.removed` `{ subject_type, subject_id, … }` | `SUBJECT_TYPE_COMMENT`: the comment is deleted with its replies (counters, realtime and `engagement.comment.deleted` as for any deletion). Any other type: everything engagement kept about the subject is purged — comments with their uploads and likes, reactions, bookmarks, share counts, counters. Either way the open reports about the subject become `ACTION_TAKEN`, and the comment texts kept for reports (of the comment and its replies, or of every comment on the subject) are forgotten. |
| `admin.report.resolved` `{ report_ids: [uuid], action_taken: bool }` | The named reports become `ACTION_TAKEN` / `NO_ACTION`. Emitted by the admin domain when a moderator resolves the case the reports were filed into. |
| `identity.user.deleted` `{ user_id }` | Erased: the user's comments (threads go with their replies), reactions, bookmarks and collections, reports they filed, screening rules, comment blocks (as owner and as blocked), mentions of them, the texts of their comments kept for reports. Purged: everything on the subjects they owned. Reports about them or their content stay, without the owner reference. Counters of other people's subjects follow, with callbacks. Works in batches of 200 per transaction. |
| `social.user.blocked` `{ user_id, blocked_id }` | Everything either of the two wrote on the other's subjects is hidden (`HIDDEN`, reason `BLOCKED_COMMENTER`) for everyone and leaves the owner's review queue: the blocked user's comments because the blocker asked not to deal with them, the blocker's own because the owner of the content could no longer see, hide or delete them. Unblocking restores nothing; the owner can approve the comments. An event that arrives after the block was lifted changes nothing. 200 comments per transaction, like account erasure (and so for `BlockCommenter`). |
| `media.asset.deleted` `{ asset_id, … }` | Safety net for forced deletions (a comment holds its image as `engagement:comment:<id>`, so owners and the janitor cannot delete it). A comment with text loses the image; a comment that was only the image is deleted. |

## Notification kinds

All through `tokuly_core::outbound::notify`, Japanese and English, inbox +
push, `subject` set to the subject, `data = { comment_id, parent_id }` for
the comment kinds. Nobody is notified about their own action, across a
block, or as the owner of content that belongs to nobody.

| Kind | To | When | Notes |
|---|---|---|---|
| `engagement.comment` | subject owner | a top-level comment entered public view | actor = commenter (none for a channel identity; channel icon as image). Path `<subject path>?comment=<id>`. |
| `engagement.comment` | subject owner | a comment is waiting for review | no actor; collapse key `engagement.review:<type>:<subject>`. Not sent for comments of blocked commenters. |
| `engagement.reply` | author of the thread; author of the reply answered | a reply entered public view | |
| `engagement.mention` | users `@mentioned` | the comment entered public view, or an edit added the mention | only users who can see the subject; at most once a day per comment and user (editing a mention out and back in is not news) |
| `engagement.heart` | comment author | the owner gave the comment a heart | no actor; collapse key `engagement.heart:<comment>`; at most once a day per comment |
| `engagement.like` | subject owner (for a comment: its author) | someone liked or emoji-reacted | actor = the user; collapse key `engagement.like:<type>:<subject>`; at most once a day per user and subject; never for dislikes |

One person gets at most one notification per comment (reply before mention
before comment). A held comment notifies its audience when it is approved,
once.

## Realtime

`EngagementEvent` on topic `subject:<type number>:<subject uuid>`
(`SubjectKey::topic`). The topic kind `subject` is guarded: only viewers for
whom the subject resolves (and who are not in a block with its owner) may
subscribe. A block and an unfriending re-check both people's `subject:`
subscriptions (`Realtime::revoke_prefix`); a subject a moderator removed
closes its topic. Deleted notes close theirs, and a note whose audience
narrows re-checks its followers (notes domain); a removal from a
community re-checks the person's `subject:` subscriptions (community
domain). Other changes
of a subject's visibility in its owning domain are caught by the gateway's
periodic re-check.

| Event | When |
|---|---|
| `comment_added` | a comment entered public view |
| `comment_updated` | a public comment was edited, hearted or lost its image; a thread's reply count changed |
| `comment_removed` | a comment left public view (deleted, hidden, held again); `removed_count` includes its replies |
| `comment_pinned` | the subject's pin moved |
| `counters_changed` | any counter of the subject changed: full totals, pushed at once when nothing was pushed during the last second, otherwise once when that second is over |

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
| `engagement.reaction` | `subject`, the subject ID | `subject_type`, `kind`, `emoji` |
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
| `engagement:liked:<user>:<type>:<subject>` | marker, 24 h | one like notification per user and subject per day |
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

Fixed rate limits: comment edits 30/min; reactions 120/min; bookmark changes
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

## Gaps and requests

- **Deleted content.** Engagement cannot tell a deleted subject from one the
  viewer may not see, so rows of content deleted by its owner stay (shown as
  unavailable in "my comments", "my reactions" and bookmarks; counted
  nowhere visible). A shared `…deleted { subject_type, subject_id }` event
  from the owning domains could reuse the purge that
  `admin.content.removed` runs (`cleanup::purge_subject`).
- **`Subjects::counter_changed` in `tokuly-core`** keeps its registry lock
  guard alive across the resolver's `.await` (a `match` on
  `self.resolvers.read().get(..).cloned()`), which makes the returned future
  `!Send`. Engagement drives the callback on a blocking thread to keep its
  handlers `Send`; binding the cloned resolver to a local before awaiting
  would remove the need.
