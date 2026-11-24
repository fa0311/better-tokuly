# social — events, jobs and configuration

Crate `tokuly-social`, migration `0200_social.sql`, API `tokuly.social.v1`
(`FriendService`, `InteractionService`, realtime `SocialEvent`).

## Events emitted

Payload structs live in `tokuly_social::events`. All are written in the same
transaction as the change they describe.

| Topic | Payload | When |
|---|---|---|
| `social.friend.requested` | `{ request_id, from_user_id, to_user_id, source }` | A friend request was stored (not when a crossed request auto-accepted). |
| `social.friend.accepted` | `{ user_id, friend_id, source }` | Two users became friends. `user_id` accepted (or completed a crossed request); `friend_id` had asked first. |
| `social.friend.removed` | `{ user_id, friend_id, blocked }` | A friendship ended. `user_id` ended it; `blocked` says it ended through a block. |
| `social.friend.prefs_changed` | `{ user_id, friend_id, muted }` | `user_id` muted or unmuted `friend_id`. Only mute changes are announced (nicknames and stars are private). Also emitted with `muted: false` when a friendship ends while one side had the other muted. The notify domain mirrors its per-friend mute list from this. |
| `social.user.blocked` | `{ user_id, blocked_id }` | `user_id` blocked `blocked_id`. If the two were friends, `social.friend.removed` (`blocked: true`) is emitted alongside. |
| `social.user.unblocked` | `{ user_id, blocked_id }` | The block was lifted. |
| `social.bump.created` | `{ bump_id, user_ids: [first, second], friends, lat, lng }` | Two users bumped. `user_ids[0]` shook first. `friends` is false for strangers (a friend request may have been sent). The geo domain may answer with `tokuly_social::bumps::set_bump_place`. |
| `social.intro.created` | `{ inviter_id, user_id }` | `user_id` signed up through `inviter_id`'s invite. |

`source` is `tokuly.social.v1.FriendSource` as its number.

Account deletion does not emit one `social.friend.removed` per friend: every
domain already reacts to `identity.user.deleted` itself.

## Events consumed

Durable consumer `social`. Every handler is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.created` | If `invite_code` belongs to someone: records the intro, suggests the two to each other, and sends a friend request from the newcomer to the inviter (source `INVITE_LINK`; the inviter's "who may send me requests" audience does not apply, since they handed out the link). |
| `identity.user.deleted` | Deletes every row about the user (friendships, requests, blocks, suggestions, invite, intros, shared history, bumps, map reactions, map views), drops cached lists, and pushes `friend_removed_user_id` to their former friends. |
| `identity.user.privacy_changed` | Re-syncs friends' view of the user's online state with `share_online_status`: hidden → pushed as offline (no "last seen"); shown again while online → pushed as online. With `nearby_discoverable` off, ends the user's open nearby session and takes them off the nearby map at once. |
| `social.friend.accepted`, `social.friend.removed`, `social.user.blocked`, `social.user.unblocked` | Our own: drops the two users' cached friend / hidden lists. The request that made the change does this right after its commit too, but it can be cut off there (deadline, lost connection); these events come from the same transaction and arrive at least once. |
| `notify.mute.changed` | `{ user_id, muted_user_id, muted }`: mirrors a mute made in the notification settings onto the friendship's `muted` flag (when the two are friends). Not re-announced as `social.friend.prefs_changed`. |

## Bus subjects (core NATS)

| Subject | Direction | Purpose |
|---|---|---|
| `sys.presence` (`tokuly_core::presence::PRESENCE_SUBJECT`) | consumed, queue group `social-presence` | Presence transitions announced by the realtime gateway. Fanned out as `tokuly.realtime.v1.PresenceEvent` to the user's friends, unless the user hides their online status. |
| `sys.social.nearby.<cell>` | published and consumed | Wake-ups between `DiscoverNearby` sessions. `<cell>` is `floor(lat*100)_floor(lng*100)`; a join or leave wakes the cell and its eight neighbours. No payload. |

## Realtime pushes

`SocialEvent`, always user-scoped:

| Event | Sent to | When |
|---|---|---|
| `friend_request_received` | recipient | A request arrived. |
| `friend_request_cancelled_id` | recipient | The sender withdrew a request the recipient could still see. |
| `friend_request_accepted` | the user who had asked | They are friends now (accept, crossed request, bump or invite completing a request). |
| `friend_removed_user_id` | both sides | Unfriend, block of a friend, or the friend's account was deleted. A block between non-friends pushes nothing (blocks are silent). |
| `bump` | both participants | A bump was recorded. Between strangers each gets their own shake's position as `location`. |
| `friends_bumped` | mutual friends of two friends who bumped | At most once per pair every 10 minutes, to at most 200 friends. Without `location`: either of the two may hide their position from the recipient (ghost mode). |
| `buzz`, `knock` | the friend | Buzz / knock. |
| `map_reaction` | each recipient | A map reaction was sent. Also stored until acknowledged. |
| `map_view` | the viewed user | A friend opened their pin (the viewer is named only if they share views). |

`ServerFrame.presence` (`PresenceEvent`) goes to the friends of a user whose
online state changed (see above).

## Notification kinds

Requested through `tokuly_core::outbound::notify`; the actor is always the
other user, so the notify domain can apply per-friend mutes.

| Kind | Recipient | Channels | Notes |
|---|---|---|---|
| `social.friend_request` | request recipient | inbox + push | Collapsed per sender. Wording depends on the source (plain, invite, bump). |
| `social.friend_accepted` | the user who had asked | inbox + push | |
| `social.bump` | both participants (friends only) | inbox only | Both are looking at their phones already. |
| `social.friends_bumped` | mutual friends | inbox + push | Collapsed per pair; `data = { bump_id, user_ids }`. |
| `social.buzz` | the friend | push only | Collapsed per sender. |
| `social.knock` | the friend | inbox + push | Collapsed per sender; `data = { count }`. |
| `social.map_reaction` | each recipient | inbox + push | Collapsed per sender. |

## Activity entries

Through `tokuly_core::outbound::activity`:

| Kind | Who | Audience | Notes |
|---|---|---|---|
| `social.friend_added` | each of the two new friends | friends — private for a user whose friend list is visible to nobody | `with_user_ids = [the new friend]` |
| `social.bumped` | each of two friends who bumped | friends | `with_user_ids = [the other]`, `data = { bump_id }`. No location: ghost mode is the geo domain's to apply. |

## Public Rust API

`tokuly_social::graph` (see the module docs):

```rust
pub async fn friend_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;          // cached in Valkey
pub async fn are_friends(core: &Core, a: Uuid, b: Uuid) -> AppResult<bool>;
pub async fn is_blocked_either(core: &Core, a: Uuid, b: Uuid) -> AppResult<bool>;
pub async fn blocked_ids_either(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;  // cached in Valkey
pub async fn relationship(core: &Core, viewer: Uuid, other: Uuid) -> AppResult<Relationship>;
pub async fn allows(core: &Core, owner: Uuid, viewer: Option<Uuid>, audience: Audience) -> AppResult<bool>;
pub async fn mutual_friend_count(core: &Core, a: Uuid, b: Uuid) -> AppResult<i32>;
pub async fn record_interaction(core: &Core, from: Uuid, to: Uuid, kind: InteractionKind, weight: i32) -> AppResult<()>;
```

- `allows`: the owner always passes; a block in either direction never does;
  anonymous viewers pass only `EVERYONE`; an unset audience counts as friends-only.
- `record_interaction` feeds the best-friend ranking. `weight` is a count for
  `Message`, `Stamp`, `Bump`, `MapView`, `MapReaction` (usually 1) and
  **minutes** for `Call` and `TimeTogether` (capped at 1440 per call).
  Interactions between non-friends are ignored. Report `TimeTogether` every
  few minutes while two friends are together rather than once at the end:
  minutes reported between 00:00 and 05:00 (reporter's time zone) count towards
  `nights_together` (one night per 60 such minutes).
- The social domain itself records `Bump`, `MapReaction` and `MapView`; chat
  should record `Message`, `Stamp` and `Call`, geo `TimeTogether`.

Also public: `tokuly_social::suggestions::suggest(core, user_id, suggested_id, SuggestionReason::NearbyOften)`
to feed "people you may know" from another domain, and
`tokuly_social::bumps::set_bump_place(core, bump_id, place_name)`.

`Social::new` registers a `tokuly_identity::ProfileExtension` (relationship,
mutual friends, friendship details, friend/intro/bump counters, relationships
in user search, profiles hidden by blocks) and a `DataExporter` named
`social`. It does not set `Profile.distance_meters`: the geo domain fills
that, with ghost mode applied.

## Background work

No job-queue jobs. `Social::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| presence fan-out | on every `sys.presence` message | See above. |
| event consumer | on events | See "Events consumed". |
| best-friend ranking (`Social::rescore_pending`) | every `TOKULY_SOCIAL_RANKING_INTERVAL_SECONDS` | Rescores pairs with new interactions, and pairs whose score is older than 6 hours (decay): 30-day window, half-life 10 days, per-side daily caps. Stores score, `interaction_score` and window titles on `friend_pairs`, then re-ranks the friends of the users involved (`friendships.best_friend_rank`). Safe to run in several processes. |
| maintenance (`Social::prune`) | every 5 minutes | Deletes interaction buckets older than 45 days, map views older than 48 hours, map reactions older than 7 days, friend requests declined more than 90 days ago, and stale members of the nearby / bump GEO sets. |

## Valkey keys

| Key | Type, TTL | Purpose |
|---|---|---|
| `social:friends:<user>` / `social:hidden:<user>` | bytes (16 per ID), 10 min | Cached friend IDs / IDs hidden by a block. Dropped on every change. |
| `social:friends:gen:<user>` / `social:hidden:gen:<user>` | counter, 1 h after the last change | Bumped by every change. A reader stores the list it loaded only if the counter has not moved since it started, so one that raced a writer cannot re-cache a stale list. |
| `social:nearby:geo`, `social:nearby:seen` | GEO set + last check-in times | Users with a `DiscoverNearby` stream open (visible for 15 s after their last heartbeat). |
| `social:nearby:session:<user>` | session ID, 11 min | The user's current `DiscoverNearby` session. A user has one at a time: an older session ends when it sees a newer one here. |
| `social:bump:geo`, `social:bump:seen` | GEO set + last check-in times | Users who just shook their phone. |
| `social:bump:attempt:<user>` | JSON, match window + 2 s | The latest shake of a user. |
| `social:bump:pair:<lo>:<hi>` | bump ID, cool-down | Makes two racing requests record one bump; later shakes within the cool-down return it. |
| `social:bump:announced:<lo>:<hi>` | marker, 10 min | Throttles "A and B bumped!" to mutual friends. |
| `social:map-view:<viewer>:<viewed>` | marker, 60 s | De-duplicates repeated map views. |

Rate limits (`rl:` keys): friend requests 60/h and 200/day per sender and
5/day per sender→recipient; contact matching 6/h and 30/day and 5000 distinct hashes a day (counted in a HyperLogLog, `rl:match-contacts-distinct:<user>`, so no hash is kept); blocks 100/h; invite
resets 10/day; invite lookups 60/min per user or IP; suggestions 60/min;
nearby sessions 20 per 10 min; bumps 30/min; buzz 10/min per pair and 60/10 min per
sender; knocks 20/min per pair and 60/10 min per sender; map reactions 120 calls/min per sender and 60/min per sender→recipient; map views 120 new views/min per viewer.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_SOCIAL_MAX_FRIENDS` | `1000` | Most friends per account (bounds every per-friend fan-out). |
| `TOKULY_SOCIAL_BEST_FRIEND_COUNT` | `8` | Friends ranked automatically per user. |
| `TOKULY_SOCIAL_RANKING_INTERVAL_SECONDS` | `300` | Pause between ranking passes. |
| `TOKULY_SOCIAL_NEARBY_RADIUS_METERS` | `50` | Nearby discovery radius (plus up to 25 m of reported GPS error). |
| `TOKULY_SOCIAL_BUMP_RADIUS_METERS` | `30` | Distance within which two shakes match (plus up to 30 m of combined GPS error). |
| `TOKULY_SOCIAL_BUMP_WINDOW_MS` | `4000` | Time within which two shakes match. |
| `TOKULY_SOCIAL_BUMP_COOLDOWN_SECONDS` | `30` | Time during which the same two people get their last bump back instead of a new one. |
| `TOKULY_SOCIAL_BUZZ_RADIUS_METERS` | `200` | How close two friends must be for a buzz. |

## Schema notes

- `friendships` holds two rows per friendship (one per direction) with each
  side's private preferences; `friend_pairs` holds what the two share
  (bump count, time together, streak, score), keyed `user_lo < user_hi`, and
  survives unfriending.
- Contact matching and "same school" suggestions look accounts up through
  identity (`tokuly_identity::users::find_by_contact_hashes` / `schoolmates`);
  the hash function and both indexes live in identity's migration.
