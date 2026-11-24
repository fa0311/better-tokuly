# stamp — events, jobs and configuration

Crate `tokuly-stamp`, migration `0500_stamp.sql`, API `tokuly.stamp.v1`
(`StampStoreService`, `StampLibraryService`, `StampStudioService`,
`StampReviewService`). Depends on `identity`, `media` and `wallet`.

## API for other domains

```rust
// Resolve stored stamp IDs for display. Deleted stamps and stamps of a pack
// that moderation took down are absent: render "no longer available".
tokuly_stamp::stamp_refs(core, &stamp_ids) -> AppResult<HashMap<Uuid, StampRef>>

// May this user send this stamp? Returns the StampRef to embed.
// Fails with STAMP_NOT_USABLE (failed_precondition), also for unknown stamps.
tokuly_stamp::ensure_usable(core, user_id, stamp_id) -> AppResult<StampRef>

// The stamp was sent. Call after the message is stored; never fails the
// message (a stamp that vanished meanwhile is ignored).
tokuly_stamp::record_send(core, user_id, stamp_id, SendContext::Chat) -> AppResult<()>
// SendContext: Chat | LiveChat | Comment | Note | Map
```

A user may send: their own personal stamps; the stamps of packs they made;
reviewed stamps of packs they installed or bought; reviewed stamps of any
published free pack, official ones included, without installing it. A paid
official pack has to be bought like any other. Nobody can send stamps of a
pack that was taken down.

## Events emitted

Payload structs live in `tokuly_stamp::events`. All are written in the same
transaction as the change they describe.

| Topic | Payload | When |
|---|---|---|
| `stamp.pack.submitted` | `{ pack_id, creator_id, official }` | A creator sent a pack to review (not when it published without review). |
| `stamp.pack.published` | `{ pack_id, creator_id, official }` | A pack went live: approved by a moderator, submitted by a trusted creator or for an official pack, resubmitted unchanged, or seeded. The actor is the moderator, the creator, or nobody (seeding). |
| `stamp.pack.rejected` | `{ pack_id, creator_id, moderator_id, reason }` | A moderator declined a pack in review. |
| `stamp.pack.unpublished` | `{ pack_id, creator_id, official }` | The creator took a pack out of the store or withdrew it from review; also when a published pack lost its last stamp to a takedown, and when a sold pack is kept after its creator's account was erased. |
| `stamp.pack.removed` | `{ pack_id, creator_id, moderator_id, reason }` | Moderation took a pack down (`admin.content.removed`). |
| `stamp.pack.deleted` | `{ pack_id, creator_id, official }` | A creator deleted a pack. Not emitted for packs erased with their creator's account. |
| `stamp.pack.installed` / `stamp.pack.uninstalled` | `{ pack_id, user_id }` | A user added a pack to / removed it from their library. Not emitted for the people who manage the pack. |
| `stamp.pack.purchased` | `{ pack_id, buyer_id, creator_id, price, net, entry_id }` | A paid pack was bought. `net` is what the creator earned after the platform fee (0 for official packs); `entry_id` is the wallet ledger entry. |
| `stamp.creator.trust_changed` | `{ user_id, trusted, moderator_id }` | A moderator changed a creator's trusted flag. |

`creator_id` is `null` for official packs. Sending a stamp emits no event
(it happens on every stamp message); see analytics below.

## Events consumed

Durable consumer `stamp`. Every handler is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.deleted` | Deletes the user's packs with their stamps, their personal stamps, and their own installs, purchases, favourites, recents, usage log rows and creator record; the uploads those stamps held are released; published packs leave the search index. Kept: a pack somebody bought — it leaves the store and the review queue and loses its unreviewed stamps, but stays in its buyers' libraries (its creator then shows as a deleted user). Untouched, because they hold their upload in their own right: copies other people saved of the user's personal stamps, and official stamps an admin uploaded. |
| `admin.content.removed` `{ subject_type, subject_id, reason, moderator_id }` | `SUBJECT_TYPE_STAMP_PACK`: the pack becomes `REMOVED` — out of the store and search, its stamps unusable and unresolvable for everyone; the creator is notified and may fix and resubmit (always reviewed). `SUBJECT_TYPE_STAMP`: the stamp is deleted together with every other stamp showing the same upload (the copies people saved); a published pack that loses its last stamp is unpublished; the owners are notified; the upload itself is then deleted by force (`tokuly_media::assets::delete(core, id, None)`). Other subject types are ignored. |
| `media.asset.deleted` `{ asset_id, … }` | Safety net for forced deletions. A stamp holds its upload, so the uploader's `DeleteAsset` is refused (`ASSET_IN_USE`) and the media janitor leaves it alone; this event reaches a stamp only when a domain deleted the upload regardless of holders (a takedown). Every stamp made from the upload is then deleted (original, saved copies, pack stamps) so nothing is left with a broken picture; affected packs get a new cover and count, and a published pack that loses its last stamp is unpublished. Nobody is notified here. This is the only way a pack somebody bought loses a stamp. |

## Media assets

Stamps serve the processed files of media assets (`ASSET_PURPOSE_STAMP`) and
keep them alive through media's usage tracking
(`tokuly_media::assets::{attach, detach}`):

| Holder | Asset | Attached | Detached |
|---|---|---|---|
| `stamp:<stamp_id>` | the upload the stamp was made from (`stamps.asset_id`) | when the stamp is stored: `AddStamps`, `CreatePersonalStamp`, and `SaveStamp` copying a personal stamp (the copy holds the same asset under its own ID) | when the stamp is deleted: `RemoveStamp`, `DeletePersonalStamp`, `DeletePack`, account deletion, takedowns |

Both happen in the transaction that stores or deletes the stamp
(`tokuly_stamp::stamps::{insert, delete, delete_of_pack}` are the only places
stamps are created and deleted). A stamp's picture is never replaced, so a
hold never moves. Pack covers are not uploads of their own: a cover is always
the picture of one of the pack's stamps and is recomputed whenever those
change, so the stamp's hold covers it and there is no cover holder. The
built-in official art is stored by the stamp domain itself and has no asset.

## Realtime pushes

None. There is no stamp event message, and nothing in the
domain is time-critical: moderation outcomes and sales arrive as
notifications, wallet changes as `WalletEvent`.

## Notification kinds

Requested through `tokuly_core::outbound::notify` (inbox + push).

| Kind | Recipient | Path | Notes |
|---|---|---|---|
| `stamp.pack.approved` | creator | `/stamps/<short_id>` | A moderator approved the pack. No actor: creators are not told which moderator decided. Not sent when a pack publishes without review. |
| `stamp.pack.rejected` | creator | `/stamps/studio/<short_id>` | Carries the moderator's reason. No actor. |
| `stamp.pack.removed` | creator | `/stamps/studio/<short_id>` | Takedown, with the reason. No actor. |
| `stamp.stamp.removed` | owner of each deleted stamp, creator of its pack | `/stamps` | A single stamp was taken down, with the reason. |
| `stamp.pack.sold` | creator | `/stamps/studio/<short_id>` | Actor is the buyer. Collapsed per pack (`stamp.pack.sold:<pack_id>`). |
| `stamp.creator.trusted` | creator | `/stamps/studio` | A moderator flagged them as trusted. |

Pack notifications carry the subject `(SUBJECT_TYPE_STAMP_PACK, pack_id)` and
the pack cover as image.

## Activity entries

| Kind | Audience | When |
|---|---|---|
| `stamp.pack.published` | public | A creator's pack is published for the first time. Subject `(SUBJECT_TYPE_STAMP_PACK, pack_id)`, path `/stamps/<short_id>`, image = cover. Never for official packs. |

## Search documents

Kind `stamp_pack`, one per published pack: `title` = name, `text` =
description, `tags`, `owner_id` = creator (absent for official packs),
`image_url` = cover, `path` = `/stamps/<short_id>`, `created_at` = publication
time, `score` = installs, `extra` = `{ short_id, price, official, animated,
stamp_count }`. Rewritten on every change to a published pack, removed when it
is unpublished, taken down or deleted.

## Subjects

| Subject type | Resolves when | Owner | Notes |
|---|---|---|---|
| `SUBJECT_TYPE_STAMP_PACK` (UUID or short ID) | the viewer may open the pack page | creator | `interactions_enabled` only while published. Keeps `like_count` and `comment_count`. |
| `SUBJECT_TYPE_STAMP` | the stamp exists for the viewer | pack creator, or the owner of a personal stamp | For reports only (`interactions_enabled` is false). |

Official packs and their stamps belong to no user: their `owner_id` is the
nil UUID. Consumers that notify a subject's owner must skip it.

## Analytics events

| Name | Subject | Value / props |
|---|---|---|
| `stamp.send` | `("stamp", stamp_id)`, owner = pack creator | `{ context, pack_id }` |
| `stamp.pack.install` | `("stamp_pack", pack_id)` | — |
| `stamp.pack.purchase` | `("stamp_pack", pack_id)`, owner = creator | value = price in coins |

## Wallet

Buying a pack calls `tokuly_wallet::ledger::transfer` (buyer → creator
earnings, fee `TOKULY_STAMP_FEE_PERCENT`) or `ledger::spend` for official
packs, with reason `stamp.pack` and the idempotency key
`stamp-pack:<pack_id>:<user_id>`. `ledger::refund` (reason
`stamp.pack.refund`) is used only when a pack disappears between the payment
and the purchase record (its creator's account was erased in that instant).

## Background work

No jobs. `Stamp::spawn_workers` starts:

| Worker | Interval | What |
|---|---|---|
| event consumer `stamp` | — | See "Events consumed". |
| ranking refresh | `TOKULY_STAMP_RANKING_INTERVAL_SECONDS` | Recomputes each active pack's popularity score (7 days; send ×1, install ×5, sale ×20; −20% per day of age) and its send counter. |
| pruning | hourly | Deletes usage log rows older than the retention period. |
| unfinished purchases | every 5 minutes | `Stamp::settle_unfinished_purchases()`: a purchase intent older than 15 minutes is paid at its stored price (or its earlier payment recognised by the ledger key) and recorded; a refused payment ends it. Intents are never swept, and a pack with one cannot be deleted. |
| official seeding | once at startup | `Stamp::seed_official_packs()` when `TOKULY_STAMP_SEED_OFFICIAL` is true. Idempotent; safe to run from several processes. |

Statistics days are counted in Japan time (`Asia/Tokyo`). The numbers
measure a pack's audience: sends by the pack's own creator and installs by
its managers are not counted, and an install undone the same day is taken
back out of that day.

## Data export

Exporter `stamp`: the user's packs with stamps, personal stamps, installed
packs, purchases, favourites and usage log.

## Configuration

| Variable (flag) | Default | Meaning |
|---|---|---|
| `TOKULY_STAMP_FEE_PERCENT` (`--stamp-fee-percent`) | `30` | Share of each pack sale kept by the platform. |
| `TOKULY_STAMP_MAX_PACKS` (`--stamp-max-packs`) | `100` | Packs per creator, all statuses. |
| `TOKULY_STAMP_MAX_PERSONAL` (`--stamp-max-personal`) | `300` | Personal stamps per user. |
| `TOKULY_STAMP_RANKING_INTERVAL_SECONDS` (`--stamp-ranking-interval-seconds`) | `300` | Seconds between ranking refreshes. |
| `TOKULY_STAMP_USAGE_RETENTION_DAYS` (`--stamp-usage-retention-days`) | `180` | Days a usage log row is kept. |
| `TOKULY_STAMP_SEED_OFFICIAL` (`--stamp-seed-official`) | `true` | Seed the built-in official packs when workers start. |

Fixed limits (`tokuly_stamp::validate`, `installs`, `collections`, `usage`):
120 stamps per pack, 30 per `AddStamps` call, paid prices 10–10,000 coins,
10 tags per pack and 10 keywords per stamp (24 characters each), 300
installed packs, 500 favourites, 64 recently used stamps.

## Storage

Official stamp art is rendered in code and stored in the media bucket under
`stamps/official/<pack_key>/<stamp_key>-v<N>.webp` (512 px) and
`…-v<N>-160.webp`, served as immutable.

## Rate limits

| Key | Limit |
|---|---|
| `stamp-search:<user or ip>` | 120 / minute (`SearchPacks`) |
| `stamp-library-search:<user>` | 300 / minute (`SearchLibraryStamps`) |
| `stamp-pack-create:<user>` | 20 / hour |
| `stamp-add:<user>` | 600 / hour (`AddStamps` calls) |
| `stamp-submit:<user>` | 30 / hour |
| `stamp-personal:<user>` | 60 / hour (`CreatePersonalStamp`) |
| `stamp-save:<user>` | 60 / minute (`SaveStamp`) |
| `stamp-buy:<user>` | 20 / minute (`PurchasePack`) |
