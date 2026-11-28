# Realtime consistency

What a screen shows is a read over HTTP plus the pushes since. The gateway's
half is in [`events/realtime.md`](events/realtime.md#frame-delivery): a read
that *starts* after `Welcome` (the user's stream) or after `onSubscribed` (a
topic) misses nothing, frames on a connection arrive in order, and delivery is
at most once. This page is the client's half, and where the apps stand on it
(audit of 2026-11-28).

## The contract

1. **Read again on every `Welcome`, the first one too, and on every
   `onSubscribed`.** A read already in flight then does not count: it is
   replaced, not joined. TanStack Query joins a fetch in flight when the query
   has no data yet, so `invalidateQueries` alone keeps a first load that
   started before the subscription. `reread` (`@tokuly/core/query`)
   replaces it; `rereadOnWelcome` calls it on every `Welcome`.
2. **A push never loses to an older read.** A push either only invalidates
   (cancel, read again: `reread`), or goes through the read journal
   (`@tokuly/core/query`, `journaled` / `pushTo`): recorded while a read of
   that query is in flight and replayed on its answer. Removals replay too, so a
   read made before a removal cannot bring the thing back. An answer to a
   mutation that writes the cache, and an add that first fetches what it adds
   (`getComment` after `commentAdded`), are reads too. Updates must be
   idempotent: add by ID, remove by ID, keep the higher version.
3. **A removed person leaves every feature.** A `friendRemovedUserId` push
   runs the same forgetters as the viewer's own block or unfriend
   (web `seams/forget-user.ts`), on both platforms.
4. **Versions only where order is lost.** Frames on one connection are in
   order, so rules 1–2 need nothing from the server. A version is needed where
   frames may be dropped or overtake each other, or one value has several
   writers: the map's `removalSequence` and snapshot tickets (droppable pins),
   wallet `revision`, chat `seq` / `readSeq`, watch-party `sequence`, the emoji
   catalogue's `version`. Where it exists, compare it on pushes and on reads.

Shared helpers in `@tokuly/core/query`: `rereadOnWelcome(queryClient,
connection, keys)` for rule 1, which cancels and refetches, and for rule 2
`journaled(queryFn)` (the query's reads) with `pushTo(queryClient, key, update)`
(every push, removals included), so a feature never wires `begin` / `finish`
by hand. State a feature keeps for a screen lives in the query cache too (the
live overlay, the watch party), so that it can use them.

## Where the apps stand

W = web, M = mobile. The issue column links the race; a dash means nothing
found.

| Area | Ordering today | What goes wrong | Issue |
|---|---|---|---|
| Map: friends' pins (W M) | snapshot tickets, `removalSequence`, sample time | presence pushed during a snapshot is lost | #538 |
| Map: gatherings, trips (W) | `setQueryData`, every `Welcome` | an ended gathering or trip, or a member who left, comes back with positions | #520 |
| Map: SOS, alerts (W) | invalidate, every `Welcome` | — | |
| Chat: conversations list (W M) | every `Welcome`, `aheadOf` by `lastSeq` | a conversation left, removed from or hidden comes back; a row's preview and unread go back | #513, #532 |
| Chat: timeline (W M) | `seq`, union by ID, catch-up by `lastSeq` | an unsent or edited message comes back from a page read before | #514 |
| Chat: unread totals, read cursors, typing, calls (W M) | version counter, forward-only cursors, `asOf` | — | |
| Notifications: inbox, badge (W M) | cancel + reread, every `Welcome` | the list's own read overwrites a newer pushed unread count | #533 |
| Friends (W M) | invalidate only, no `Welcome` handling | W: an ex-friend or a blocker stays in the list, a withdrawn request in the badge | #527 |
| Notes: timelines (W M) | hints on every `Welcome`, a look after the first page | — | |
| Notes: card topic (W M) | no read on `onSubscribed` | a note deleted or edited before its topic took effect stays as it was | #526 |
| Notes: forgetting a person (W M) | `setQueryData`, no journal | an ex-friend's or a blocker's notes come back from a page read before | #515 |
| Comments (W) | `onSubscribed` invalidates, `setQueriesData` | a removed comment comes back; replies are never read again | #521 |
| Community (W M) | invalidate, `onSubscribed` on the community | a removed reply comes back | #522 |
| Moments (W M) | `setQueryData`, every `Welcome` | a deleted moment, or an ex-friend's, comes back; W never forgets a person who unfriends | #517, #528 |
| Stories (W M) | `setQueryData`, every `Welcome` | a deleted story, or an ex-friend's reel, comes back in the tray | #516 |
| Plans (W M) | `setQueryData(null)` on removal, every `Welcome` | a plan the person was removed from, or a deleted poll, comes back | #518 |
| Shorts (W M) | W `resetQueries` | M: an ex-friend's friends-only shorts keep playing | #529 |
| Spaces: hangouts (W M) | `setQueryData`, every `Welcome` | a hangout the person was removed from comes back with who is inside | #519 |
| Spaces: rooms (W M) | `onSubscribed` invalidates | an ended room shows LIVE again; someone who left is back on stage | #530 |
| Live: chat (W M) | journal, `onSubscribed` | — | |
| Live: stream page (W M) | `onSubscribed` reread, `setQueryData` | a deleted message comes back as the pin or a highlight; a pin change goes back | #523 |
| Live: overlay (W) | journal | a revoked widget comes back, frozen | #540 |
| Watch party, video cards (W M) | party: journal + `sequence` | heartbeat playback lost to a reload; a pushed card overwritten | #539 |
| School: room (W) | journal, `onSubscribed` | — | |
| School: own session (W M) | timer `serverTime` | a stopped session comes back until the heartbeat fails | #531 |
| Wallet: balance (W M) | `revision`, every `Welcome` | — | |
| Wallet: ledger, purchases, payouts (W M) | `setQueryData`, every `Welcome` | a new entry or status is overwritten by an older page | #534 |
| Ask (W M) | `setQueryData`, every `Welcome` | a new question drops out of the inbox and badge | #535 |
| Quest (W M) | write + invalidate, every `Welcome` | a level up goes back to the old level | #536 |
| Emoji catalogue (W M) | `version`, `onSubscribed` sync | the sync on `onSubscribed` joins one read before it | #537 |
| Engagement, counters (W) | `setQueryData`, every `Welcome` | counts go back to an older read | #541 |
| Footprints, history (W) | invalidate / `resetQueries` | — | |

## Adopting it, smallest steps first

1. Done: `rereadOnWelcome` in `@tokuly/core/query`, used by every feature
   that reads again on `Welcome` (#524); notes timelines look after a hint
   during the first load on both platforms (#525).
2. Topic reads replace a read in flight: notes cards, emoji (#526, #537).
3. The missing removal handlers: web moments, mobile shorts, web friends
   (#527, #528, #529).
4. `journaled` / `pushTo` (done; used by live chat, the live overlay, the
   watch party and the school room), then each area that writes with
   `setQueryData`, privacy first: chat, notes, stories, moments, plans,
   hangouts, gatherings, comments, community, live pins; counters last.
