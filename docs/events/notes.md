# notes — events, jobs and configuration

Crate `tokuly-notes`, migration `0700_notes.sql`, API `tokuly.notes.v1`
(`NoteService`, `StoryService`, realtime `NotesEvent`). Depends on `identity`,
`social`, `stamp` and `media`.

Notes are short posts (「今何してる?」); stories are 「記憶」: a title plus 1–10
photos or videos that the owner's friends see for a day and that stay in the
owner's archive afterwards.

## Who may see what

Every read goes through two SQL conditions in `tokuly_notes::visibility`
(`sees_note`, `sees_story`); there is no second implementation. The
audiences are the social graph's (`tokuly_social::graph::ContentAudience`,
loaded once per request as `AudienceViewer`).

| | visible to |
|---|---|
| any note | its author, always |
| PUBLIC note | everyone, signed in or not — but a **locked** author's public notes only their followers and friends |
| FOLLOWERS (6) | the author's followers and friends |
| FRIENDS (4) | the author's friends |
| CLOSE_FRIENDS (7) | members of the author's close-friends circle (while still a friend or follower) |
| CIRCLE (8) | members of that custom circle of the author (same); others are shown the note as FOLLOWERS, so members never learn about the circle |
| direct (PRIVATE naming people) | the people it names (1–20) |
| PRIVATE | nobody else |
| a reply narrower than PUBLIC (not PRIVATE) | also the author of the note it answers |
| story | its owner (the archive); the owner's friends while it is live or for as long as it is in a highlight — a close-friends story only the friends chosen for it |

Nobody sees anything across a block (either direction), and nobody else sees
content of a suspended or deleted account. A reply is never shown more widely
than the note it answers (narrowed when posted and when its audience is
changed): an answer to a PUBLIC, FOLLOWERS or FRIENDS note takes the narrower
of the two; an answer to a close-friends, circle or direct note becomes a
direct note to its author (and the people it named); an answer to a private
note is private. Ties and blocks are read from `tokuly_social::graph` on
every request, so an unfollow, an unfriend or a block takes effect at once.

**Listed notes.** A note is `listed` when everyone may see it: public, and
its author not locked. Only listed notes are in search, in hashtag counts
and trends, in the global timeline, quotable, and renotable by people other
than the author (the author may renote their own followers, friends, close
friends and circle notes; a renote never shows the note to anyone outside
its audience). Whether an author is locked is copied from the social domain
into `note_authors` (on `social.follow.settings_changed`, and when they
post); reads ask the social domain again about listed rows shown to someone
not tied to the author, so a lock applies at once.

Web paths used in notifications, activity entries, search documents and
reports: `/notes/<note_id>`, `/notes/tags/<tag>`, `/stories/<story_id>`.

## Events emitted

Payload structs live in `tokuly_notes::events`. All are written in the
transaction that makes the change.

| Topic | Payload | When |
|---|---|---|
| `notes.note.created` | `{ note_id, author_id, visibility, reply_to_id, quote_of_id }` | A note, reply or quote note was posted. `visibility` is `tokuly.common.v1.Visibility` as its number (1 public, 4 friends, 5 private). |
| `notes.note.deleted` | `{ note_id, author_id, reason }` | A note is gone for good. `reason`: `author`, `moderation`, `account_deleted`, `media_deleted`. Actor: the author, the moderator, the erased user, or nobody. |
| `notes.note.visibility_changed` | `{ note_id, author_id, visibility, visible_to, changed_at }` | The author changed who may see a note (not emitted when nothing changed), or locked or unlocked their account (once per public note whose reach changed). `visibility`: the audience now in effect — the note's own, except that a public note of a locked author reads 6 (followers). `visible_to`: who besides the author may still see it — `null` when everyone may; otherwise everyone in the audience as the social graph gives it (followers and friends, friends, close friends or circle members, the people a direct note names) and, for a reply narrower than public, the author of the note answered, people in a block with the author left out; `[]` for a private note. For a change caused by a lock, only the people this domain told about the note or who reacted to it are considered (a full follower list per note would be unbounded). Anyone shown the note before (an inbox entry about it) who is not listed lost access with this change. `changed_at`: when the change was made, taken while the note was locked; consumers (feed, notify) order two changes of one note by it, not by the event's time (the start of its transaction). **Never forward this event to webhooks or other outside parties: `visible_to` is the author's friend list.** |
| `notes.story.created` | `{ story_id, owner_id, audience, expires_at }` | A story was posted. `audience` is `tokuly.notes.v1.StoryAudience` as its number (1 friends, 2 close friends). |
| `notes.story.deleted` | `{ story_id, owner_id, reason }` | A story was deleted (same reasons as notes). Not emitted for an expiry. |
| `notes.story.expired` | `{ story_id, owner_id }` | A story's day is over: it left the tray and is archive-only. Emitted exactly once per story by the expiry worker, shortly after `expires_at`. |

Likes, renotes and views emit no domain event (they happen constantly); see
"Analytics" below. The feed domain can consume `notes.note.deleted` /
`notes.story.deleted` to drop activity entries of content that is gone.

## Events consumed

Durable consumer `notes` (`tokuly_notes::Notes::handle_event`). Every handler
is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` `{ user_id }` | Deletes the user's notes and stories with everything on them (uploads released, search documents removed, hashtag counts put right, `notes.note.deleted` / `notes.story.deleted` with reason `account_deleted`), their likes, renotes, story views and reactions (the counters they added go down again), their highlights and rules agreement, and takes them out of other people's mentions, delivery records (`note_recipients`) and close-friends audiences. Replies other people wrote to the erased notes stay and show what they answered as unavailable. |
| `admin.content.removed` `{ subject_type, subject_id, reason, moderator_id }` | `SUBJECT_TYPE_NOTE` / `SUBJECT_TYPE_STORY`: the content is deleted like its author would delete it (reason `moderation`, actor = the moderator). Nobody is notified here: the admin domain tells the owner. Unknown or already removed subjects and other subject types are ignored. |
| `media.asset.deleted` `{ asset_id, … }` | Safety net for forced deletions (notes and stories hold their uploads, so owners cannot delete those and the janitor leaves them alone). The picture or video is taken out of the notes and stories that show it; a note left with nothing (no text, no stamp, no other media) and a story left without items are deleted (reason `media_deleted`). Public notes get a fresh search document. |
| `social.follow.settings_changed` `{ user_id, locked }` | Re-reads whether the account is locked (late or repeated events give the same result), stores it in `note_authors` and re-lists the account's public notes in batches of 200: out of (or back into) search, hashtag counts and the global timeline, with one `notes.note.visibility_changed` per note that changed. |
| `social.user.blocked` `{ user_id, blocked_id }` | Nothing has to be hidden (every read asks the social domain who is blocked). The handler removes the one standing grant between the two: being among the chosen friends of the other's close-friends stories. |

## Realtime pushes

`NotesEvent`.

Topic `subject:3:<note_id>` (3 = `SUBJECT_TYPE_NOTE`; the topic kind `subject`
is the engagement domain's, whose guard lets a client subscribe only to a note
it can see at that moment):

| Event | When |
|---|---|
| `counters_changed` (`NoteCounters`) | Likes, renotes, replies or quotes of the note changed. Views alone do not push; the view count rides along with the next change. |
| `note_deleted_id` | The note was deleted. |
| `link_preview_resolved` | The preview of the note's first link was fetched. To the topic only when the note is listed; always to the author (user-scoped). |

User-scoped:

| Event | Sent to | When |
|---|---|---|
| `story_posted` (`Story`) | everyone the story is shown to (all friends, or the chosen friends), as they will see it; the owner's devices get the owner's view | A story was posted. |
| `story_deleted` (`StoryRef`) | the same people, if the story was still live; always the owner | A story was deleted. |
| `story_viewed` (`StoryViewCount`) | the owner | Someone watched the story for the first time. |
| `story_reaction` (`StoryReaction`) | the owner | Someone reacted, or changed their reaction. |

## Notification kinds

All in the `notes` preference category. The actor is always the other user,
so per-friend mutes apply. Only people who may see the note are notified, and
each person at most once per note (reply, then quote, then mention).

| Kind | Recipient | Notes |
|---|---|---|
| `notes.like` | note author | Collapsed per note (`notes.like:<note_id>`). The same person liking, unliking and liking again within 24 h is announced once. |
| `notes.renote` | note author | Collapsed per note and renoter; announced once per 24 h like likes. |
| `notes.reply` | author of the note answered | Body is the reply's text. |
| `notes.quote` | author of the quoted note | |
| `notes.mention` | each mentioned user | At most 10 mentions per note are resolved. People in a block with the author are not mentioned at all. |
| `notes.direct` | each person a direct note names | Not repeated when the audience is changed later. |
| `notes.story_reaction` | story owner | Collapsed per story and person: a changed emoji replaces the earlier notification. `data = { story_id, emoji }`. |

`subject` is `(SUBJECT_TYPE_NOTE, note_id)` or `(SUBJECT_TYPE_STORY, story_id)`;
`path` opens the note (for replies, quotes and mentions: the new note) or the story.

## Activity entries

| Kind | Audience | Notes |
|---|---|---|
| `note.posted` | public for a listed note; friends for a friends or followers note and for a locked author's public note (friends are in those audiences); private otherwise | Notes and quote notes. `data = { note_id, text, reply_to_id, quote_of_id, content_warning }`; image = first picture unless the note carries a content warning; location = the place tag. |
| `note.replied` | matches the reply | Same data; `with_user_ids = [author of the note answered]`. |
| `story.posted` | friends — private for a close-friends story (the feed cannot know who was chosen) | `data = { story_id, title, item_count, expires_at }`; image = the story's cover. |

## Search documents

| Kind | ID | Indexed while | Notes |
|---|---|---|---|
| `note` | note ID | the note is listed (public, author not locked; replies included) | `title` = one-line excerpt, `text`, `tags` = hashtags, `owner_id` = author, `score` = likes + 2·renotes + replies + quotes at indexing time, `extra = { author_handle, author_name, reply_to_id, content_warning, attachment_count }`. Removed when the note is deleted or stops being listed. |
| `hashtag` | hashtag ID | at least one listed note carries it | `title` = `#tag`, `score` = public note count, `path` = `/notes/tags/<tag>`. The trending worker re-sends documents whose count moved since they were last indexed. |

## Subjects, profiles, export

- `SUBJECT_TYPE_NOTE` and `SUBJECT_TYPE_STORY` resolvers, visibility-aware.
  For reports (`resolve_for_report`) they also resolve what reached the
  reporter before it was hidden from them (made private, a block, an
  expired story), so withdrawing does not take away the report: a public
  note, a note they were told about (reply, quote or mention notification,
  kept in `note_recipients`), a note they liked or renoted, a story they
  watched.
  Notes: `interactions_enabled = true` (likes and replies are native; the
  generic domains add emoji reactions, bookmarks, shares and reports).
  Stories: `false` (own reactions; replies are DMs), still reportable.
  `counter_changed` is not implemented: every counter of a note is this
  domain's own.
- `ProfileExtension`: fills `ProfileStats.note_count` with the owner's notes
  (replies aside) that the viewer may see.
- `DataExporter` named `notes`: rules agreement, notes, likes, renotes,
  notes announced to the user (`received_notes`), stories, story views,
  highlights.

## Other domains' APIs used

- `tokuly_media::assets::{require_ready, attach, detach_holder}` — holders
  `notes:note:<note_id>` and `notes:story:<story_id>`; purposes
  `NOTE_IMAGE` / `NOTE_VIDEO` (notes) and `STORY` (stories).
- `tokuly_media::link_preview` for the first link of a note (background job).
- `tokuly_stamp::{ensure_usable, record_send(.., SendContext::Note), stamp_refs}`.
- `tokuly_social::graph::{AudienceViewer, ContentAudience, audience_member_ids, ensure_circle_of,
  locked_among, following_ids, muted_followee_ids, friend_ids, blocked_ids_either, record_interaction}`.
  Interactions between friends are reported with weight 1: a reply as
  `InteractionKind::Message`, a like or renote as `InteractionKind::MapReaction`
  (the social domain has no kind of its own for note reactions).
- `tokuly_identity::settings::privacy_of` (`default_note_visibility`),
  `summaries`, and read-only lookups on `users` (handles for mentions, time
  zone for the story calendar, account status).

## Background work

`Notes::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| event consumer | on events | See "Events consumed". |
| job `notes.link_preview` | payload `LinkPreviewJob { note_id }`, concurrency 4, 3 attempts | Fetches the preview of a note's first link, stores it and pushes `link_preview_resolved` (`Notes::resolve_link_preview`). A link with nothing to show leaves the note without a preview. |
| story expiry (`Notes::expire_stories`) | every `TOKULY_NOTES_STORY_EXPIRY_INTERVAL_SECONDS` | Marks stories past `expires_at` as archived and emits `notes.story.expired` once each. Visibility never depends on this pass. |
| trending hashtags (`Notes::refresh_trends`) | every `TOKULY_NOTES_TREND_INTERVAL_SECONDS` | Recomputes `hashtag_trends` (top 50): hashtags used in public notes by at least `TOKULY_NOTES_TREND_MIN_AUTHORS` different people within the window; each person counts once per hashtag, by their latest use, halved every 6 hours. Also refreshes stale hashtag search documents. Safe to run in several processes (advisory lock). |

## Analytics

`core.analytics.track`: `note.post` (props: visibility, reply, quote,
attachments), `note.like`, `note.renote` (owner = note author), `story.post`
(props: items, close_friends), `story.view` (owner = story owner).

## Valkey keys

| Key | TTL | Purpose |
|---|---|---|
| `notes:idem:note:<user>:<key>` / `notes:idem:story:<user>:<key>` | 10 min | Idempotency keys of `CreateNote` / `CreateStory` → the ID created. |
| `notes:view:<note>:<viewer>` | `TOKULY_NOTES_VIEW_WINDOW_HOURS` | One view per viewer (user ID, or `ip:<address>` for anonymous viewers) and note within the window. |
| `notes:notified:<like\|renote>:<note>:<user>` | 24 h | Keeps repeated likes/renotes of one person from being announced again. |

Rate limits (`rl:` keys): visibility changes 30/min per user;
posting `TOKULY_NOTES_POSTS_PER_MINUTE` per minute
and 1000 per day per user; likes and renotes 120/min (taking one back is never refused);
view reports 120/min per viewer (excess reports are ignored, not refused);
stories `TOKULY_NOTES_STORIES_PER_HOUR` per hour; story reactions 60/min;
hashtag completion 120/min per user or IP.

## Configuration (`NotesConfig`)

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_NOTES_STORY_TTL_HOURS` | `24` | Hours a story stays visible to friends. |
| `TOKULY_NOTES_POSTS_PER_MINUTE` | `30` | Notes (replies and quotes included) one user may post per minute. |
| `TOKULY_NOTES_STORIES_PER_HOUR` | `30` | Stories one user may post per hour. |
| `TOKULY_NOTES_VIEW_WINDOW_HOURS` | `6` | Hours within which repeated views by one viewer count once. |
| `TOKULY_NOTES_TREND_INTERVAL_SECONDS` | `300` | Pause between trending recomputations. |
| `TOKULY_NOTES_TREND_WINDOW_HOURS` | `24` | Hours of public notes the ranking looks at. |
| `TOKULY_NOTES_TREND_MIN_AUTHORS` | `2` | Different people needed for a hashtag to trend. |
| `TOKULY_NOTES_STORY_EXPIRY_INTERVAL_SECONDS` | `60` | Pause between story expiry passes. |

The posting rules (`GetNoteRules`) are text in `tokuly_notes::rules`, Japanese
and English; raising `RULES_VERSION` there asks everyone to agree again before
their next note.

## Limits

Note: 300 characters (user-perceived; at most 40 bytes each on average), 4
images or 1 video, 10 hashtags and 10 mentions resolved per note, place name
100 characters. Story: title 140, 1–10 items, caption 200, at most 150 chosen
friends. Highlight: title 1–30, 1–100 stories, 50 highlights per profile.

## Public Rust API

```rust
impl Notes {
    pub fn new(core: Core, identity: Identity, cfg: NotesConfig) -> anyhow::Result<Self>;
    pub fn register(&self, router: connectrpc::Router) -> connectrpc::Router;
    pub fn spawn_workers(&self);
    /// For the demo simulator: posts a public-by-default note, recording the
    /// author's agreement to the rules and skipping the posting rate limits.
    pub async fn post_note(&self, author: Uuid, text: &str) -> AppResult<Uuid>;
    // Worker entry points, public for tools and tests:
    pub async fn handle_event(&self, event: Event) -> AppResult<()>;
    pub async fn refresh_trends(&self) -> AppResult<usize>;
    pub async fn expire_stories(&self) -> AppResult<u64>;
    pub async fn resolve_link_preview(&self, note_id: Uuid) -> AppResult<()>;
}
```

## Schema notes

- A note is immutable apart from its visibility, pin and counters. Replies
  and quotes point at their target without a foreign key, so they outlive it.
- `note_likes.id` / `note_renotes.id` are time-ordered and are the keyset of
  "who liked this" and "what did this user like"; a renote's ID also places it
  among notes in timelines. A note appears once per timeline: where its latest
  renote by someone in the list's scope puts it, or at its own position.
- `hashtags.note_count` counts listed notes only; `note_hashtags` links every
  note, so friends-only notes still appear in a hashtag timeline for the
  people who may see them.
- `story_views` holds one row per viewer: when they watched and their
  reaction (`''` = none). `stories.archived_at` only records that the expiry
  was announced.

## Lists

`NoteListService` (`lists.rs`, `list_svc.rs`): named sets of accounts with a
timeline of their own (members' notes and renotes plus the replies among
members, through the same visibility condition as every other timeline,
leaving out members the viewer blocked or is blocked by and accounts the
viewer muted as a followee). Channel notes appear only for readers who
follow the channel, as on the following timeline.

- **Who sees a list**: the owner always; anyone (signed in or not) when it is
  public and its owner is an account in good standing that no block hides
  from them. A private list is `LIST_NOT_FOUND` for everyone else.
  `member_count` and `ListListMembers` count only members the viewer may
  see (no block either way, account active).
- **Blocks**: nobody is added across a block (`USER_NOT_FOUND`, as for a
  missing account). `social.user.blocked` takes the two off each other's
  lists and list follows (follower counts go down) and out of each other's
  USERS antennas. Adding re-checks the block after commit, so a block that
  lands while members are added is still applied.
- **Locked accounts**: on a public list only when the owner follows them
  (an approved follower) or is their friend (`MEMBER_NOT_ALLOWED`); on a
  private list, anyone. A private list made public drops the locked members
  that would not be allowed and tells the others they are on it.
- A list made private loses its followers (`follower_count` = 0).
- **Account deletion**: the account's lists (members, follows) and antennas
  are deleted, it leaves every list, its list follows are removed (counts
  go down) and other people's antennas stop naming it.
- **Export**: `notes.lists` (with member IDs), `notes.list_follows`.

| Notification kind | Recipient | Notes |
|---|---|---|
| `notes.list_added` | the person added to a **public** list (not the owner themself) | Collapsed per list (`notes.list_added:<list_id>`), path `/notes/lists/<id>`, `data = { list_id, list_name }`. Once per person and list a day; at most 100 per owner and hour (people are still added beyond that, just not told). |

Limits: 50 lists per owner, 500 members per list, 50 accounts per
add/remove call, 200 followed lists per person; name 1–50 characters,
description 200. The list and member limits hold under concurrent calls
(per-owner advisory lock; the list row is locked while members are added).

Rate limits: `notes:list-create:<user>` 30/hour; `notes:list-change:<user>`
(updates and member adds) 60/min; `notes:list-follow:<user>` 60/min;
`notes:list-notices:<owner>` 100/hour (silently stops notifying).

Valkey keys: `notes:notified:list:<list>:<user>` (24 h) — the person was told
about this list.

## Antennas

`AntennaService` (`antennas.rs`, `antenna_svc.rs`): saved searches that
collect new notes into a timeline only their owner sees.

- **Matching** happens in the job `notes.antenna_match`
  (`posting::JOB_ANTENNA_MATCH`, payload `{ note_id }`, queued once per new
  note; concurrency 4, lease 60 s, 5 deliveries) through
  `Notes::match_antennas(note_id) -> AppResult<usize>`. Its cost does not
  grow with the number of antennas: one in-process Aho-Corasick automaton
  over the folded words of all antennas (NFKC + lowercase, blank runs as one
  space) finds every word in the note's text and content warning in one
  pass; EVERYONE antennas become candidates only through a word hit;
  FOLLOWING ones from the author's followers and friends, LIST ones from the
  lists the author is on (`note_list_members_user` index), USERS ones from an
  in-memory index of the accounts they name. Each candidate's owner is then
  checked once (`Viewer::can_see_note`, blocks both ways, the account lock
  the social domain knows now, owner account active).
- A note matches when every word of one keyword group appears and no
  excluded word does; replies only with `with_replies`, media only with
  `media_only`; notes older than the antenna are not collected late. LIST
  and USERS antennas may have no keywords (everything of those accounts).
- Collected notes go into `antenna_notes`; the latest 1000 per antenna are
  kept. The job is idempotent (`ON CONFLICT DO NOTHING`; nothing is
  announced twice). The timeline shows only collected notes the owner may
  still see.
- The automaton is rebuilt when `notes:antennas:version` (Valkey counter,
  `INCR` on every antenna change, list deletion, block or account erasure
  touching antennas) moves, at least every 5 minutes, and every 30 s while
  Valkey cannot be read.
- Deleting a list deletes the LIST antennas built on it (foreign key).
- **Export**: `notes.antennas`.

| Notification kind | Recipient | Notes |
|---|---|---|
| `notes.antenna` | antenna owner, when `notify` is on | Collapsed per antenna (`notes.antenna:<antenna_id>`), at most one per antenna every 10 minutes; actor = the note's author; path `/notes/antennas/<id>`; `data = { antenna_id, note_id }`. |

Limits: 5 antennas per person (holds under concurrent creation), name 1–30
characters, 10 keyword groups of 5 words, 10 excluded words, words of at
most 50 characters, 1–50 accounts for USERS.

Errors beyond the proto's list: `INVALID_USERS` (USERS antenna with no or
more than 50 accounts) and `INVALID_SOURCE` (an unknown source value).

Rate limits: `notes:antenna-change:<user>` 60/hour (creates and updates).

Valkey keys: `notes:antennas:version` (no TTL) — the antenna version
counter; `notes:notified:antenna:<antenna>` (10 min) — the owner was just
told about this antenna.
