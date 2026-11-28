# Domain events

Events flow through the transactional outbox into the `EVENTS` JetStream
stream (`tokuly_core::events`). Topics are `<domain>.<entity>.<verb>`; payloads
are JSON. Each domain lists what it emits and consumes under its heading.

An event too large for the bus is dropped by the relay and logged (ID, topic,
size); producers keep events small (`visible_to` lists are bounded by
`tokuly_core::events::MAX_VISIBLE_TO`).

A consumer retries a failing event with a growing delay (twelve deliveries,
about twenty minutes), a job worker up to the job's `max_deliver`. What
still fails, or cannot be decoded, is kept on the `DEAD` stream, payload
included, for `TOKULY_DEAD_LETTER_RETENTION_DAYS` (7 by default, at most 30)
with the consumer it failed in and the last error. Once the cause is fixed:

```sh
docker compose run --rm api dead-letters list            # what is buried, where it failed and why
docker compose run --rm api dead-letters redrive         # publish everything again
docker compose run --rm api dead-letters redrive notify  # only what failed in consumers named notify…
```

A redriven event is handled only by the consumer it failed in (header
`Tokuly-Redrive-For`); the others handled it long ago and ack it unread.

## Changing a payload

Events stay on the stream for 7 days and dead letters for up to 30, and a
redrive sends a letter as it was buried: new code reads payloads written by
releases a month old. So a payload only grows.

- **Add fields, never rename or retype one.** A new field is optional for its
  readers (`#[serde(default)]`, or an `Option`): old events do not have it.
- **Remove a field over two releases.** The first stops reading it and keeps
  writing it; the second, at least 30 days later, stops writing it.
- **A different meaning is a new topic** (`…v2` or a new verb), emitted next to
  the old one until nothing consumes the old one.
- Readers ignore fields they do not know (serde's default; no
  `deny_unknown_fields` on payload structs), so a rollback reads new events.

When an account is erased, identity removes every letter that mentions its
ID, except `identity.user.deleted` itself: that one holds only the ID, and
redriving it is how a consumer that failed still erases its part.

Three topics are generic requests handled by the generic domains
(`tokuly_core::outbound`):

| Topic | Payload | Handled by |
|---|---|---|
| `notify.requested` | `NotificationRequest` | notify |
| `activity.recorded` | `ActivityEntry` | feed |
| `search.changed` | `SearchChange` | search |

One topic is shared by every domain that owns subjects
(`tokuly_core::outbound::subject_deleted` / `subjects_deleted`):

| Topic | Payload | Handled by |
|---|---|---|
| `subject.deleted` | `SubjectDeleted { subject_type, subject_id }` (`tokuly.common.v1.SubjectType` as its number, the subject's UUID) | engagement |

A domain emits it in the transaction that deletes a subject for good,
whoever deleted it, so that the generic domains drop what they attached to
it. The rules:

- **Gone, not hidden.** Content made private, unpublished, narrowed to an
  audience or put behind a block has its own events
  (`…visibility_changed` with `visible_to` and `changed_at`,
  `…unpublished`), never this one.
- **Not while it answers reports.** A domain that keeps removed content for
  a while so the people it reached can still report it emits when that time
  is over and the row goes. Content that takes no comments (a chat message)
  may be announced at once.
- **Next to the domain's own deletion event, not instead of it**: notify and
  feed withdraw by `notes.note.deleted`, `video.video.deleted` and the
  others.
- No actor; delivered at least once; consumers treat a subject they know
  nothing about as done.

Each domain's file says when it emits it.

## identity

See `docs/events/identity.md`.

## wallet (ledger)

Emits:

| Topic | Payload |
|---|---|
| `wallet.balance.changed` | `{ user_id, coins, earnings, entry_id, reason }` |

Other domains add their own file under `docs/events/<domain>.md`.
