# social — events, jobs and configuration

Crate `tokuly-social`, migration `0200_social.sql`, API `tokuly.social.v1`
(`FriendService`, `InteractionService`, `FollowService`, `CircleService`,
realtime `SocialEvent`).

## Friends, follows and circles

Three relationships, none of which implies another:

- **Friendship**: mutual, requested and accepted, with the map, location
  and everything private. Becoming friends follows nobody.
- **Follow**: one way, for public content (notes, shorts, videos, lives,
  communities). Following someone gives no access to their location.
  A **locked** account approves its followers (requests, declined silently;
  unlocking approves every request still waiting). Without a stored setting
  an account is locked exactly when its owner is a minor by birthdate.
- **Circles**: the built-in close-friends list and up to 20 custom ones
  (name, one emoji, colour, order; 1000 members each). Members are the
  owner's friends and followers only — a circle is an audience for what the
  owner shares, so only people with a tie to the owner belong in it, and
  that rules out lists of strangers. Someone whose tie ends (unfriended and
  not following, unfollowed, removed as a follower, a block, a deleted
  account) is taken out of all of the owner's circles in the same
  transaction. Members are never told and no RPC answers "which circles am
  I in"; only content shared with close friends may show its viewers a mark.

Blocks end friendships, follows (both ways), follow requests (both ways)
and circle memberships (both ways), and keep new ones out. Follow lists and
counts show active accounts only (not deleted, not suspended) and leave out
people in a block with the viewer, so a count always matches its list. The
lists of a locked account are shown to the owner, its followers and its
friends.

As audiences (see `tokuly_social::audience`, below): `Followers` means
followers **and** friends, the closer tier; a locked owner's `Public`
content only reaches their followers and friends.

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
| `social.follow.created` | `{ follower_id, followee_id, approved }` | A follow began. `approved`: through an approved request (or unlocking). |
| `social.follow.removed` | `{ follower_id, followee_id, reason }` | A follow ended. `reason`: `unfollowed`, `removed` (the followee removed the follower) or `blocked` (one blocked the other; one event per direction that existed). |
| `social.follow.requested` | `{ request_id, follower_id, followee_id }` | Someone asked to follow a locked account. |
| `social.follow.request_withdrawn` | `{ request_id, follower_id, followee_id }` | A request the followee could still answer is gone unanswered (withdrawn, unfollowed, or dropped by a block). Not emitted for declined requests (declining is silent) or approvals (`social.follow.created`). |
| `social.follow.prefs_changed` | `{ follower_id, followee_id, muted, muted_until, notify_level }` | The follower changed their mute (optionally timed: `muted_until`) or post notifications (`FollowNotifyLevel` number) for one account. Private to the follower. A timed mute ends by itself without an event. |
| `social.follow.settings_changed` | `{ user_id, locked }` | The account was locked or unlocked. |
| `social.circle.created` / `social.circle.updated` | `{ circle_id, owner_id, kind }` | A custom circle was made; a circle was renamed or recoloured. `kind` is `CircleKind` as its number. The close-friends circle is made silently on first use. |
| `social.circle.deleted` | `{ circle_id, owner_id, member_ids }` | A custom circle was deleted; content shared with it is the owner's alone. |
| `social.circle.members_changed` | `{ circle_id, owner_id, added, removed }` | People were added or taken out (by the owner, or because their tie ended). Never shown to members. |

`source` is `tokuly.social.v1.FriendSource` as its number.

Account deletion does not emit one `social.friend.removed` per friend (nor
`social.follow.removed` per follow): every domain already reacts to
`identity.user.deleted` itself.

## Events consumed

Durable consumer `social`. Every handler is idempotent.

| Topic | Reaction |
|---|---|
| `identity.user.created` | If `invite_code` belongs to someone: records the intro, suggests the two to each other, and sends a friend request from the newcomer to the inviter (source `INVITE_LINK`; the inviter's "who may send me requests" audience does not apply, since they handed out the link). |
| `identity.user.deleted` | Deletes every row about the user (friendships, requests, blocks, suggestions, invite, intros, shared history, bumps, map reactions, map views, follows, follow requests, follow settings, dismissed follow suggestions, their circles and their memberships in others'), drops cached lists, and pushes `friend_removed_user_id` to their former friends. |
| `identity.user.privacy_changed` | Re-syncs friends' view of the user's online state with `share_online_status`: hidden → pushed as offline (no "last seen"); shown again while online → pushed as online. With `nearby_discoverable` off, ends the user's open nearby session and takes them off the nearby map at once. |
| `social.friend.accepted`, `social.friend.removed`, `social.friend.prefs_changed`, `social.user.blocked`, `social.user.unblocked`, `social.follow.created`, `social.follow.removed`, `social.circle.members_changed`, `social.circle.deleted` | Our own: drops the cached lists they changed (friends and friendship details, hidden, followers / following, circle members and memberships). The request that made the change does this right after its commit too, but it can be cut off there (deadline, lost connection); these events come from the same transaction and arrive at least once. |
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
| `new_follower` | the followee | Someone started following them (an open account). |
| `follow_request_received` | the followee | Someone asked to follow their locked account. |
| `follow_request_cancelled_id` | the followee | A request they could still answer was withdrawn (also when they blocked its sender). |
| `follow_request_approved` | the requester | Their request was approved (or the account was unlocked). |

Removing a follower, declining a request and every circle change push
nothing to the other side.

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
| `social.new_follower` | the followee | inbox + push | Collapsed per follower; leads to the follower's profile. |
| `social.follow_request` | the locked account | inbox + push | Collapsed per requester; leads to `/friends/follow-requests`. |
| `social.follow_approved` | the requester | inbox + push | Leads to the approver's profile. |

## Activity entries

Through `tokuly_core::outbound::activity`:

| Kind | Who | Audience | Notes |
|---|---|---|---|
| `social.friend_added` | each of the two new friends | friends — private for a user whose friend list is visible to nobody | `with_user_ids = [the new friend]` |
| `social.bumped` | each of two friends who bumped | friends | `with_user_ids = [the other]`, `data = { bump_id }`. No location: ghost mode is the geo domain's to apply. |

Follows and circles write no activity entries: who follows whom is shown on
lists people choose to look at, not announced to friends.

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

Friendship details (cached per user, dropped on every friendship,
preference or rank change):

```rust
pub struct FriendLink { pub friend_id: Uuid, pub since: DateTime<Utc>, pub best_friend_rank: i32, pub favorite: bool, pub muted: bool }
pub async fn friend_links(core: &Core, user_id: Uuid) -> AppResult<Vec<FriendLink>>;
pub async fn friend_link(core: &Core, user_id: Uuid, friend_id: Uuid) -> AppResult<Option<FriendLink>>;
pub async fn friends_since(core: &Core, a: Uuid, b: Uuid) -> AppResult<Option<DateTime<Utc>>>;
pub async fn friends_since_many(core: &Core, user_id: Uuid, others: &[Uuid]) -> AppResult<HashMap<Uuid, DateTime<Utc>>>;
pub async fn best_friend_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;   // ranked in order, then starred
pub async fn muted_friend_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;
pub async fn muted_by(core: &Core, friend_id: Uuid, user_ids: &[Uuid]) -> AppResult<Vec<Uuid>>;  // reverse direction, one query
```

Follows:

```rust
pub async fn follows(core: &Core, follower: Uuid, followee: Uuid) -> AppResult<bool>;   // PostgreSQL
pub async fn follower_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;         // cached; any account state
pub async fn following_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;        // cached
pub async fn muted_followee_ids(core: &Core, user_id: Uuid) -> AppResult<Vec<Uuid>>;   // for following timelines
pub async fn post_notification_recipients(core: &Core, owner: Uuid) -> AppResult<Vec<Uuid>>; // bell on, not muted, active, no block
```

Audiences (`tokuly_social::audience`, re-exported from `graph`) — what content
domains ask instead of each writing their own rules:

```rust
pub enum ContentAudience { Public, Followers, Friends, CloseFriends, Circle(Uuid), Private }
impl ContentAudience { pub fn from_visibility(v: Visibility) -> Self }   // PUBLIC/NO_FEED/UNLISTED → Public, FRIENDS, PRIVATE
pub async fn may_see(core: &Core, owner: Uuid, viewer: Option<Uuid>, audience: ContentAudience) -> AppResult<bool>;  // PostgreSQL
pub async fn may_see_many(core: &Core, viewer: Option<Uuid>, items: &[(Uuid, ContentAudience)]) -> AppResult<Vec<bool>>;
pub struct AudienceViewer;  // load(core, viewer) once, then sees(owner, audience, owner_locked); friend_ids(), following_ids(), hidden_ids(), circle_ids(), close_friend_of() for SQL
pub async fn locked_among(core: &Core, owners: &[Uuid]) -> AppResult<HashSet<Uuid>>;
pub async fn audience_member_ids(core: &Core, owner: Uuid, audience: ContentAudience) -> AppResult<Vec<Uuid>>;  // fan-out
pub async fn ensure_circle_of(core: &Core, owner: Uuid, circle_id: Uuid) -> AppResult<()>;  // CIRCLE_NOT_FOUND; check when saving
pub async fn is_close_friend(core: &Core, owner: Uuid, viewer: Uuid) -> AppResult<bool>;    // for the close-friends mark
```

| audience | who sees it (besides the owner; never across a block) |
|---|---|
| `Public` | everyone, signed in or not; for a locked owner only followers and friends |
| `Followers` | followers and friends |
| `Friends` | friends |
| `CloseFriends` / `Circle(id)` | members of that circle of the owner who are still a friend or follower |
| `Private` | nobody |

The owner's account state (suspended, deleted), link-only access and age
limits stay with the content domain. `tokuly.common.v1.Visibility` has
`VISIBILITY_FOLLOWERS`, `VISIBILITY_CLOSE_FRIENDS` and `VISIBILITY_CIRCLE`
(with `AudienceRef.circle_id`); `ContentAudience::from_visibility(v, circle_id)`
/ `from_stored(i32, circle_id)` map them (unknown → `Private`). No content
domain honours them yet: live, video and shorts refuse them with
`VISIBILITY_INVALID` until they decide visibility through this API.

Follow suggestions take more signals through
`Social::add_follow_signal(Arc<dyn tokuly_social::follow_suggestions::FollowSignal>)`:
a reason string, a weight and `candidates(core, user_id) -> Vec<(Uuid, strength)>`;
the score is `Σ weight × ln(1 + strength)`. Built in: `FOLLOWS_YOU` (3),
`FRIEND_OF_FRIEND` (2), `FOLLOWED_BY_FOLLOWING` (1.5), `POPULAR_AT_SCHOOL` (1,
at least 3 unlocked schoolmates). The community domain's
`POPULAR_IN_COMMUNITY` is meant to be plugged in by the server.

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

### Claimed positions (`tokuly_social::claims`)

`DiscoverNearby` and `Bump` are matched on a position the client sends. It
is checked first by a `claims::PositionCheck`, which the geo domain plugs in
(`Geo::new` calls `Social::set_position_check`): does the account's own
latest real fix (recent, accurate to 100 m, not mocked, moving at ground
speed or settled after a jump) put it there? The check answers plausible,
not verified or too coarse about the caller's own claim, never a position
or distance, so social holds nothing derived from a true position.
What others are shown stays computed from the claim (distance bands, one's
own shake position, the point between two friends' shakes); the true
position only decides whether the claimer takes part.

A refused claim is refused, not moved to the real position, for both
features: social does not have the real position, and a session or bump
placed there would show strangers (and friends of both, for a bump) something
derived from it, which ghost mode forbids for whoever does not see the person
precisely.

| Feature | A claim that fails the check |
|---|---|
| `DiscoverNearby` | The stream is refused with `LOCATION_NOT_VERIFIED` (`LOCATION_TOO_COARSE` while the device only gives approximate locations); the user never appears on the nearby map. |
| `Bump` | The shake is refused with the same reasons before it is published, so nobody can match it; the other phone sees no bump. |

Without a check plugged in, every claim is refused (fail closed). The geo
side and the two-world test showing that what Ren sees does not depend on
where the claimer really is: `docs/events/geo.md` ("Claimed positions") and
`crates/geo/tests/claims.rs`.

`Social::new` registers a `tokuly_identity::ProfileExtension` (relationship,
mutual friends, friendship details, friend/intro/bump counters, relationships
in user search, profiles hidden by blocks, and for signed-in viewers a
`social.follow` card: also `ProfileStats.follower_count` / `following_count`, and
`data = { follower_count, following_count, following:
NONE|FOLLOWING|REQUESTED, follows_you, locked, known_follower_count }`, path
`/u/<handle>/followers`) and a `DataExporter` named `social` (which also
holds follows both ways, follow requests, follow settings, dismissed follow
suggestions and the user's own circles with their members — not the circles
of others the user is in). It does not set `Profile.distance_meters`: the geo domain fills
that, with ghost mode applied.

## Background work

No job-queue jobs. `Social::spawn_workers` starts:

| Worker | Schedule | What it does |
|---|---|---|
| presence fan-out | on every `sys.presence` message | See above. |
| event consumer | on events | See "Events consumed". |
| best-friend ranking (`Social::rescore_pending`) | every `TOKULY_SOCIAL_RANKING_INTERVAL_SECONDS` | Rescores pairs with new interactions, and pairs whose score is older than 6 hours (decay): 30-day window, half-life 10 days, per-side daily caps. Stores score, `interaction_score` and window titles on `friend_pairs`, then re-ranks the friends of the users involved (`friendships.best_friend_rank`). Safe to run in several processes. |
| maintenance (`Social::prune`) | every 5 minutes | Deletes interaction buckets older than 45 days, map views older than 48 hours, map reactions older than 7 days, friend and follow requests declined more than 90 days ago (counted from the decline), and stale members of the nearby / bump GEO sets. |

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
| `social:friend-links:<user>` | 29 bytes per friend, 10 min | Friendship details for `graph::friend_links`. Generation counter like the others. |
| `social:followers:<user>` / `social:following:<user>` | bytes (16 per ID), 10 min | Cached follower / followee IDs. |
| `social:circle-members:<circle>` / `social:member-of:<user>` | bytes, 10 min | Members of a circle; the circles a user is in (33 bytes each: circle, owner, close-friends flag). Server-side only. |

Rate limits (`rl:` keys): friend requests 60/h and 200/day per sender and
5/day per sender→recipient; contact matching 6/h and 30/day and 5000 distinct hashes a day (counted in a HyperLogLog, `rl:match-contacts-distinct:<user>`, so no hash is kept); blocks 100/h; invite
resets 10/day; invite lookups 60/min per user or IP; suggestions 60/min;
nearby sessions 20 per 10 min; bumps 30/min; buzz 10/min per pair and 60/10 min per
sender; knocks 20/min per pair and 60/10 min per sender; map reactions 120 calls/min per sender and 60/min per sender→recipient; map views 120 new views/min per viewer; follows 200/h and 1000/day per
follower and 10/day per follower→followee; removing followers 300/h; follow
preferences 600/h; follow settings 30/h; follow suggestions 60/min; creating
circles 30/day; circle member changes 600 calls/h; other circle edits 300/h.

## Configuration

| Variable | Default | Meaning |
|---|---|---|
| `TOKULY_SOCIAL_MAX_FRIENDS` | `1000` | Most friends per account (bounds every per-friend fan-out). |
| `TOKULY_SOCIAL_MAX_FOLLOWING` | `5000` | Most accounts one user can follow, pending follow requests included (so an approval never goes over). |
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
