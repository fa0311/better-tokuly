# Domain events

Events flow through the transactional outbox into the `EVENTS` JetStream
stream (`tokuly_core::events`). Topics are `<domain>.<entity>.<verb>`; payloads
are JSON. Each domain lists what it emits and consumes under its heading.

Three topics are generic requests handled by the generic domains
(`tokuly_core::outbound`):

| Topic | Payload | Handled by |
|---|---|---|
| `notify.requested` | `NotificationRequest` | notify |
| `activity.recorded` | `ActivityEntry` | feed |
| `search.changed` | `SearchChange` | search |

## identity

Emits:

| Topic | Payload |
|---|---|
| `identity.user.created` | `{ user_id, handle, invite_code }` |
| `identity.user.updated` | `{ user_id }` (profile fields changed) |
| `identity.user.status_changed` | `{ user_id }` |
| `identity.user.privacy_changed` | `{ user_id }` |
| `identity.user.deletion_requested` | `{ user_id }` |
| `identity.user.deleted` | `{ user_id }` — every domain removes that user's data |
| `identity.user.banned` / `identity.user.unbanned` | `{ user_id }` |
| `identity.export.ready` | `{ user_id, export_id }` |

## wallet (ledger)

Emits:

| Topic | Payload |
|---|---|
| `wallet.balance.changed` | `{ user_id, coins, earnings, entry_id, reason }` |

Other domains add their own file under `docs/events/<domain>.md`.
