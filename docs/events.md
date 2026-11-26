# Domain events

Events flow through the transactional outbox into the `EVENTS` JetStream
stream (`tokuly_core::events`). Topics are `<domain>.<entity>.<verb>`; payloads
are JSON. Each domain lists what it emits and consumes under its heading.

The relay drops (and logs, with the event's ID, topic and size) a row too
large for the bus to take, instead of retrying it and holding back every
row after it; producers keep events small (`visible_to` lists are bounded
by `tokuly_core::events::MAX_VISIBLE_TO`).

A consumer retries an event its handler fails on with a growing delay
(2 s doubling to 5 minutes, twelve deliveries: about twenty minutes), a job
worker likewise up to the job's `max_deliver`. What still fails after the
last attempt, or cannot be decoded, is not dropped: it is buried on the
`DEAD` stream for 30 days with the consumer it failed in and the last error
(`tokuly_core::dead_letters`). Once the cause is fixed, an operator sends
it round again:

```sh
docker compose run --rm api dead-letters list            # what is buried, where it failed and why
docker compose run --rm api dead-letters redrive         # publish everything again
docker compose run --rm api dead-letters redrive notify  # only what failed in consumers named notify…
```

A redriven event goes back on its topic with a `Tokuly-Redrive-For` header
naming the consumer it failed in; every other consumer of the topic acks it
unread. They handled it weeks ago, and what they remember of handled events
(notify and feed keep event IDs for 8 days) is shorter than a letter is
kept, so a redrive to everyone would notify people again of old news.

Three topics are generic requests handled by the generic domains
(`tokuly_core::outbound`):

| Topic | Payload | Handled by |
|---|---|---|
| `notify.requested` | `NotificationRequest` | notify |
| `activity.recorded` | `ActivityEntry` | feed |
| `search.changed` | `SearchChange` | search |

## identity

See `docs/events/identity.md`.

## wallet (ledger)

Emits:

| Topic | Payload |
|---|---|
| `wallet.balance.changed` | `{ user_id, coins, earnings, entry_id, reason }` |

Other domains add their own file under `docs/events/<domain>.md`.
