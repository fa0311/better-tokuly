# emoji — events, jobs and configuration

Crate `tokuly-emoji`, migration `1700_emoji.sql`, API `tokuly.emoji.v1`
(`EmojiService`, `EmojiSubmissionService`, `EmojiAdminService`). Depends on
`identity` and `media`.

Custom emoji (カスタム絵文字) are small images with a `:shortcode:`, modelled
on Misskey's: a unique name (`a-z0-9_`, 2–32 characters, matched without
regard to case), a picture from a media upload (still or animated), a
category from a managed list, up to 16 aliases, a licence note, an author,
and the flags `sensitive` and `local_only`.

```
           submit                    approve
 (nothing) ───────▶ PENDING ──────────────────────▶ APPROVED ◀──┐
     │                 │  ▲                           │   ▲     │ enable
     │ trusted / staff │  │ author changes it         │   │     │
     └─────────────────┼──┼───────────────────────────┘   │  DISABLED
                reject ▼  │                    disable /  │     ▲
                     REJECTED ─── approve ────────────────┘     │
                                               takedown ────────┘
```

A name is held by every emoji that is not REJECTED. Everything in
`EmojiAdminService` needs the `moderator` role (admins hold every role).

## API for other domains

```rust
// Is there an approved emoji with this name? (any case, with or without colons)
tokuly_emoji::catalog::exists(core: &Core, name: &str) -> AppResult<bool>

// The images of the approved emoji among `names`, keyed by the name as it
// was passed in. Unknown and unapproved names are absent. `Image.url` is the
// animated WebP of an animated emoji and the largest still otherwise;
// `Image.variants` are stills by longest edge.
tokuly_emoji::catalog::resolve(core: &Core, names: &[String]) -> AppResult<HashMap<String, Image>>

// The same with the whole catalogue entry (id, category, aliases, sensitive, local_only…).
tokuly_emoji::catalog::lookup(core: &Core, names: &[String]) -> AppResult<HashMap<String, CustomEmoji>>

// A user used these emoji, once each: "recently used" and "popular" follow.
// Call after the message or reaction is stored. Unicode emoji may be listed
// too (remembered as recently used, not counted); anything else is ignored.
tokuly_emoji::catalog::record_use(core: &Core, user_id: Uuid, names: &[String]) -> AppResult<()>
```

All four are answered from the cached catalogue: one small Valkey read per
call and no database query (`record_use` adds one statement for the user's
recently used list and one Valkey script). They are meant to be called on
every request. A domain that may not depend on this crate reaches them
through a bridge in the server crate (`backend/crates/server/src/bridges.rs`).

## The catalogue and its versions

The catalogue is the set of APPROVED emoji plus the list of categories. It
has a version that grows by one with every change to either. Clients keep a
copy: `GetCatalog` once, then `GetCatalogChanges(since_version)` when the
topic `emoji:catalog` announces a newer version or the app starts.

- The counter is the single row of `emoji_catalog`. A change takes the next
  version as the **last step of its transaction**; whoever updates the row
  holds it until commit, so versions are handed out in commit order.
- Each approved emoji carries the version of its last change
  (`emojis.catalog_version`); each emoji that left the catalogue leaves a row
  in `emoji_catalog_removals`. "What changed since N" is a filter on both.
- Removals are forgotten after `TOKULY_EMOJI_REMOVAL_RETENTION_DAYS`, which
  raises `floor_version`. A client holding a version below the floor, above
  the current one (another database) or zero gets the whole catalogue with
  `full = true`.
- Clients key their copy by emoji **ID**: a rename keeps it.

### Cache

| Valkey key | Holds |
|---|---|
| `emoji:<epoch>:catalog:version` | the current version |
| `emoji:<epoch>:catalog:data` | the serialized catalogue at that version (JSON, 1 h TTL, rewritten on demand) |
| `emoji:<epoch>:catalog:announced` / `:gate` | the last version announced; a 1 s gate between announcements |
| `emoji:<epoch>:uses`, `:uses:draining` | use counters waiting for / being written by the usage worker (hash: emoji ID → uses) |
| `emoji:<epoch>:uses:<yyyymmdd>:<user>` | one user's counted uses of the day (2 days TTL) |

`<epoch>` is an ID created with the database (`emoji_catalog.epoch`), so a
Valkey shared by several databases never mixes their catalogues.

Each process decodes the catalogue once per version and keeps it in memory;
a reader compares the version in Valkey with the one it holds. After a
change commits, the version in Valkey is raised (which drops the serialized
copy) and the new version is announced. The maintenance worker repeats both
every minute, for the case that a process stopped between commit and
announcement, and resets the cache if the database was restored to an
earlier version.

## Events emitted

Payload structs live in `tokuly_emoji::events`. All are written in the same
transaction as the change they describe.

| Topic | Payload | When |
|---|---|---|
| `emoji.emoji.submitted` | `{ emoji_id, name, author_id }` | An emoji entered the review queue: a new submission, or a rejected one submitted again. Actor: the author. |
| `emoji.emoji.rejected` | `{ emoji_id, name, author_id, moderator_id, reason }` | A moderator declined a waiting emoji. |
| `emoji.emoji.added` | `{ emoji_id, name, author_id, via }` | An emoji joined the catalogue. `via`: `review` (approved by a moderator), `trusted` (submitted by a trusted submitter or a moderator), `staff` (added directly), `import`, `enabled` (a disabled emoji put back). Actor: the moderator, or the author for `trusted`. |
| `emoji.emoji.updated` | `{ emoji_id, name, previous_name? }` | An emoji in the catalogue changed. `previous_name` is set when it was renamed. Not emitted for category renames (the catalogue version covers those). |
| `emoji.emoji.removed` | `{ emoji_id, name, author_id, because }` | An emoji left the catalogue. `because`: `disabled`, `taken_down` (`admin.content.removed`), `deleted`, `asset_deleted`. |
| `emoji.submitter.trust_changed` | `{ user_id, trusted, moderator_id }` | A moderator changed a submitter's trusted flag. |

`author_id` is `null` for imported emoji, uncredited additions and emoji
whose author's account is gone. Using an emoji emits no event.

## Events consumed

Durable consumer `emoji` (`tokuly_emoji::Emoji::handle_event`). Every handler
is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` `{ user_id }` | The user's PENDING and REJECTED emoji are deleted (their uploads released), as are their deck, their recently used list and their trusted flag. Their APPROVED and DISABLED emoji stay, with the credit cleared and the search document rewritten without owner. Moderation history, imports, exports and trust flags they set keep their rows with the user reference nulled. |
| `admin.content.removed` `{ subject_type, subject_id, reason, moderator_id }` | `SUBJECT_TYPE_CUSTOM_EMOJI`: an approved emoji becomes DISABLED (out of the catalogue and the search index, name kept), with the reason as review note and a `TAKEN_DOWN` history entry. No notification: the admin domain tells the owner. Unknown emoji, emoji that are not approved and other subject types are ignored. A moderator can enable the emoji again. |
| `media.asset.ready` / `media.asset.failed` `{ asset_id, purpose, … }` | If the upload is the picture of an import entry that waits for it, the entry is settled: READY → the emoji is created, FAILED → the entry fails with media's reason. Uploads of other purposes are skipped without a query. |
| `media.asset.deleted` `{ asset_id, … }` | Safety net for forced deletions. An emoji holds its upload, so its uploader's `DeleteAsset` is refused and the janitor leaves it alone; when a domain deletes the upload regardless, every emoji showing it is deleted (in any state) so nothing is left with a broken picture. |

## Jobs

| Job | Payload | What |
|---|---|---|
| `emoji.import` | `{ import_id }` | Reads an uploaded pack: plans every entry, creates missing categories, hands the pictures to the media domain. Resumable; a pack that cannot be read fails the import without retry. Concurrency 1, 3 attempts. |
| `emoji.export` | `{ export_id }` | Writes the catalogue (or one category) to a ZIP in the private bucket. Marks the export FAILED instead of retrying. Concurrency 1. |

## Import and export

Packs are ZIP archives in Misskey's emoji export format: `meta.json`
(`metaVersion`, `host`, `exportedAt`, `emojis: [{ fileName, downloaded,
emoji: { name, category, aliases, license, isSensitive, localOnly, … } }]`)
next to the image files it names. Reading is forgiving (missing or `null`
fields, a pack zipped as one folder); writing produces every field Misskey
writes, so an export can be imported by a Misskey server.

Import (`StartImport`, upload with `ASSET_PURPOSE_CHAT_FILE`):

1. The job plans each entry. Names are lowered and `-`, `+`, `.` and spaces
   become `_`. A taken name is skipped or numbered (`name_2`, `name_3`, …)
   according to the import's policy; a pack clashing with itself counts too.
2. Each accepted entry's image is unpacked (size checked while unpacking)
   and stored as an upload of the importing moderator
   (`tokuly_media::create_asset_from_file`, `ASSET_PURPOSE_STAMP`).
3. When media has processed a picture, the entry is settled: the emoji is
   created APPROVED (history `IMPORTED`, no author). If the planned name was
   taken meanwhile, the policy applies again. Pictures of entries that end
   skipped or failed are deleted.
4. The import is COMPLETED when its last entry is settled.

Entry outcomes: `IMPORTED`, `RENAMED`, `SKIPPED` (`NAME_TAKEN`,
`DUPLICATE_NAME`), `FAILED` (`NAME_INVALID`, `NOT_DOWNLOADED`,
`FILE_NAME_INVALID`, `FILE_MISSING`, `FILE_UNREADABLE`, `IMAGE_TOO_LARGE`,
`IMAGE_TOO_SMALL`, `UNSUPPORTED_CONTENT`, media's processing reason,
`PROCESSING_TIMEOUT`). Import failures: `ARCHIVE_INVALID`, `META_MISSING`,
`META_INVALID`, `TOO_MANY_ENTRIES`, `INTERNAL`.

Export (`StartExport`): the pictures go into the archive as they were
uploaded (the originals in the private bucket); an emoji whose original
cannot be read is listed with `downloaded: false`. The archive is stored
under `emoji/exports/<export_id>.zip` in the private bucket; `GetExport`
returns a presigned link valid for one hour, and the file is deleted after
`TOKULY_EMOJI_EXPORT_TTL_HOURS`.

Not carried over from Misskey: role-restricted reactions
(`roleIdsThatCanBeUsedThisEmojiAsReaction` is ignored on import and written
as `[]`).

## Media assets

| Holder | Asset | Attached | Detached |
|---|---|---|---|
| `emoji:<emoji_id>` | the picture (`emojis.asset_id`, `ASSET_PURPOSE_STAMP`: a still image or an animation) | when the emoji is stored (`SubmitEmoji`, `AddEmoji`, import) or gets a new picture | when the picture is replaced or the emoji is deleted (withdrawal, `DeleteEmoji`, account deletion) |
| `emoji:import:<import_id>` | the uploaded pack (`ASSET_PURPOSE_CHAT_FILE`) | `StartImport` | when the import completes or fails |

Both happen in the transaction that stores or deletes the reference.

## Realtime pushes

`EmojiEvent` (`ServerFrame.emoji`):

| Event | To | When |
|---|---|---|
| `catalog_changed { version }` | public topic `emoji:catalog` | The catalogue has a newer version. Announcements are bundled: at most about one a second, always ending with the latest version. |
| `submission_updated` (`EmojiDetail`) | the author | A moderator approved, rejected, disabled, enabled or edited one of their emoji. |
| `import_updated` (`Import`) | the moderator who started it | Every 25 settled entries, and when it completes or fails. |
| `export_updated` (`Export`) | the moderator who started it | When it is ready or failed. |

## Notification kinds

Requested through `tokuly_core::outbound::notify` (inbox + push), without
actor: authors are not told which moderator decided.

| Kind | Recipient | Path | Notes |
|---|---|---|---|
| `emoji.approved` | author | `/emoji/<name>` | A moderator approved the emoji. Not sent when an emoji skips review. |
| `emoji.rejected` | author | `/emoji/submissions` | Carries the moderator's reason. |
| `emoji.disabled` | author | `/emoji/submissions` | A moderator disabled the emoji, with the reason. Not sent for takedowns (the admin domain sends `system.content_removed`). |
| `emoji.trusted` | submitter | `/emoji/submissions` | A moderator flagged them as trusted. |

Emoji notifications carry the subject `(SUBJECT_TYPE_CUSTOM_EMOJI, emoji_id)`,
the emoji's picture as image and `data = { emoji_id, name }`. The notify
domain has no category for the `emoji.` prefix yet, so they land in `system`.

## Activity entries

| Kind | Audience | When |
|---|---|---|
| `emoji.added` | public | An emoji with a credited author joins the catalogue for the first time (not when it is enabled again). Subject `(SUBJECT_TYPE_CUSTOM_EMOJI, emoji_id)`, path `/emoji/<name>`, image = the emoji. |

## Search documents

Kind `emoji`, one per approved emoji: `title` = name, `text` = category,
`tags` = aliases, `owner_id` = author (absent when uncredited), `image_url`,
`path` = `/emoji/<name>`, `created_at` = when it joined the catalogue,
`extra` = `{ name, category, animated, sensitive }`. Rewritten on every
change to an approved emoji (including a rename of its category and the
deletion of its author's account), removed when it is disabled or deleted.
The search domain does not know the kind yet (it needs a `SEARCH_KIND_EMOJI`
and a kind spec); until then these changes are ignored there. `SearchEmoji`
does not depend on it: it searches the cached catalogue.

## Subjects

| Subject type | Resolves when | Owner | Notes |
|---|---|---|---|
| `SUBJECT_TYPE_CUSTOM_EMOJI` (emoji ID or name) | the emoji is approved, or the viewer is its author | author; the nil UUID when nobody is credited | For reports only (`interactions_enabled` is false). Title `:name:`, path `/emoji/<name>`. |

## Profile card

`tokuly_identity::ProfileExtension`: people with emoji in the catalogue get
a card of kind `emoji.author` — title 作った絵文字 / Emoji made, text
`N個` / `N emoji`, the newest emoji's picture, path `/emoji?author=<user_id>`,
`data = { count, newest }` (`newest`: up to five shortcodes, newest first),
order 60. Authorship is public (it is on the emoji's page), so the card is
shown to everyone who may see the profile.

## Data export

Exporter `emoji`: the user's emoji in every state, their deck, their
recently used list and whether they are a trusted submitter.

## Background work

`Emoji::spawn_workers` starts:

| Worker | Interval | What |
|---|---|---|
| event consumer `emoji` | — | See "Events consumed". |
| jobs `emoji.import`, `emoji.export` | — | See "Jobs". |
| usage worker | `TOKULY_EMOJI_USAGE_FLUSH_SECONDS` | Moves the use counters from Valkey into `emoji_usage_daily` and `emoji_stats` in batches that are applied exactly once (`emoji_usage_batches`), then recomputes popularity: the uses of the last `TOKULY_EMOJI_POPULAR_DAYS` days, each day weighing 0.85 of the day after it. |
| maintenance | every minute | Reconciles the catalogue cache and its announcements with the database; settles import entries whose upload finished without its event being handled; fails entries and imports stuck for six hours; deletes expired export files. |
| pruning | hourly | Forgets removals past their retention (raising the floor version), daily counters older than 400 days and batch markers older than a day. |

Statistics days are counted in Japan time (`Asia/Tokyo`). One user counts at
most 20 times per emoji and day towards "popular".

## Configuration

| Variable (flag) | Default | Meaning |
|---|---|---|
| `TOKULY_EMOJI_MAX_PENDING` (`--emoji-max-pending`) | `10` | Submissions one user may have waiting for review. |
| `TOKULY_EMOJI_MAX_IMAGE_KB` (`--emoji-max-image-kb`) | `1024` | Largest upload accepted as an emoji picture (also for imported pictures). |
| `TOKULY_EMOJI_RESERVED_NAMES` (`--emoji-reserved-names`) | empty | Extra names only moderators may give, comma separated, on top of the built-in list (`admin`, `official`, `tokuly`, …). |
| `TOKULY_EMOJI_DEFAULT_DECK` (`--emoji-default-deck`) | `👍,❤️,😆,🤔,😮,🎉,💢,😥,😇,🍮` | The deck of users who never saved one. |
| `TOKULY_EMOJI_USAGE_FLUSH_SECONDS` (`--emoji-usage-flush-seconds`) | `60` | Seconds between runs of the usage worker. |
| `TOKULY_EMOJI_POPULAR_DAYS` (`--emoji-popular-days`) | `7` | Days of use that count towards "popular". |
| `TOKULY_EMOJI_REMOVAL_RETENTION_DAYS` (`--emoji-removal-retention-days`) | `90` | Days the catalogue remembers a removal. |
| `TOKULY_EMOJI_IMPORT_MAX_MB` (`--emoji-import-max-mb`) | `100` | Largest pack accepted for import. |
| `TOKULY_EMOJI_IMPORT_MAX_ENTRIES` (`--emoji-import-max-entries`) | `5000` | Most entries an imported pack may have. |
| `TOKULY_EMOJI_EXPORT_TTL_HOURS` (`--emoji-export-ttl-hours`) | `168` | Hours an export file can be downloaded. |

Fixed limits (`tokuly_emoji::validate`): names 2–32 characters; 16 aliases
of 32 characters; licence notes of 1024 characters; 200 categories of 32
characters; reasons of 500 characters; queries of 64 characters; decks of
100 emoji; 64 recently used emoji; 50 emoji per `RecordUse`; 200 emoji per
bulk change; pictures of at least 16 pixels on the longer side.

## Storage

Export archives: `emoji/exports/<export_id>.zip` in the private bucket.
Emoji pictures are the media domain's renditions; nothing else is stored.

## Rate limits

| Key | Limit |
|---|---|
| `emoji-catalog:<user or ip>` | 60 / minute (`GetCatalog`) |
| `emoji-changes:<user or ip>` | 240 / minute (`GetCatalogChanges`) |
| `emoji-search:<user or ip>` | 300 / minute (`SearchEmoji`) |
| `emoji-deck:<user>` | 60 / minute (`SetDeck`) |
| `emoji-use:<user>` | 120 / minute (`RecordUse`) |
| `emoji-name:<user>` | 120 / minute (`CheckName`) |
| `emoji-submit:<user>` | 20 / hour (`SubmitEmoji`) |
| `emoji-edit:<user>` | 60 / hour (`UpdateSubmission`) |
| `emoji-import:<moderator>` | 20 / hour (`StartImport`) |
| `emoji-export:<moderator>` | 20 / hour (`StartExport`) |
